//! FFmpeg-backed audio validation and two-pass WAV normalization.
//!
//! This module deliberately uses the safe `ffmpeg-next` wrappers throughout.
//! No raw `ffmpeg::ffi` calls are currently necessary; if that changes, they
//! belong in a single documented `ffi` module rather than at call sites.

use std::fs;
use std::path::{Path, PathBuf};

use ffmpeg::{Rescale, codec, filter, format, frame, media};
use ffmpeg_next as ffmpeg;
use tracing::info;

const EAGAIN: ffmpeg::Error = ffmpeg::Error::Other {
    errno: ffmpeg::error::EAGAIN,
};

/// Packed PCM formats supported by the WAV encoder allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    U8,
    S16,
    S32,
    S64,
    F32,
    F64,
}

impl SampleFormat {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::S16 => "s16",
            Self::S32 => "s32",
            Self::S64 => "s64",
            Self::F32 => "flt",
            Self::F64 => "dbl",
        }
    }

    fn ffmpeg(self) -> format::Sample {
        use format::sample::Type::Packed;
        match self {
            Self::U8 => format::Sample::U8(Packed),
            Self::S16 => format::Sample::I16(Packed),
            Self::S32 => format::Sample::I32(Packed),
            Self::S64 => format::Sample::I64(Packed),
            Self::F32 => format::Sample::F32(Packed),
            Self::F64 => format::Sample::F64(Packed),
        }
    }

    fn codec(self) -> codec::Id {
        match self {
            Self::U8 => codec::Id::PCM_U8,
            Self::S16 => codec::Id::PCM_S16LE,
            Self::S32 => codec::Id::PCM_S32LE,
            Self::S64 => codec::Id::PCM_S64LE,
            Self::F32 => codec::Id::PCM_F32LE,
            Self::F64 => codec::Id::PCM_F64LE,
        }
    }
}

impl std::str::FromStr for SampleFormat {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "u8" => Ok(Self::U8),
            "s16" => Ok(Self::S16),
            "s32" => Ok(Self::S32),
            "s64" => Ok(Self::S64),
            "flt" | "f32" => Ok(Self::F32),
            "dbl" | "f64" => Ok(Self::F64),
            _ => Err(Error::InvalidOptions(format!(
                "unsupported sample format `{value}`; expected u8, s16, s32, s64, flt, or dbl"
            ))),
        }
    }
}

/// Targets and tolerances for [`normalize`].
#[derive(Debug, Clone)]
pub struct NormalizeOptions {
    pub offset_seconds: f64,
    pub sample_format: SampleFormat,
    pub sample_rate: u32,
    pub loudness_lufs: f64,
    pub loudness_range_lu: f64,
    pub true_peak_dbtp: f64,
    pub true_peak_tolerance_db: f64,
    pub loudness_range_tolerance_lu: f64,
    pub gain_tolerance_db: f64,
    pub offset_tolerance_seconds: f64,
}

impl Default for NormalizeOptions {
    fn default() -> Self {
        Self {
            offset_seconds: 0.0,
            sample_format: SampleFormat::S16,
            sample_rate: 48_000,
            loudness_lufs: -8.25,
            loudness_range_lu: 11.0,
            true_peak_dbtp: 0.0,
            true_peak_tolerance_db: 0.5,
            loudness_range_tolerance_lu: 0.1,
            gain_tolerance_db: 0.2,
            offset_tolerance_seconds: 0.000_1,
        }
    }
}

impl NormalizeOptions {
    fn validate(&self) -> Result<()> {
        let finite = [
            self.offset_seconds,
            self.loudness_lufs,
            self.loudness_range_lu,
            self.true_peak_dbtp,
            self.true_peak_tolerance_db,
            self.loudness_range_tolerance_lu,
            self.gain_tolerance_db,
            self.offset_tolerance_seconds,
        ]
        .into_iter()
        .all(f64::is_finite);
        if !finite {
            return Err(Error::InvalidOptions(
                "all numeric options must be finite".into(),
            ));
        }
        if self.sample_rate == 0 {
            return Err(Error::InvalidOptions(
                "sample rate must be greater than zero".into(),
            ));
        }
        if self.loudness_range_lu < 0.0
            || self.true_peak_tolerance_db < 0.0
            || self.loudness_range_tolerance_lu < 0.0
            || self.gain_tolerance_db < 0.0
            || self.offset_tolerance_seconds < 0.0
        {
            return Err(Error::InvalidOptions(
                "ranges and tolerances must not be negative".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub codec: String,
    pub sample_format: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessStats {
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
    pub target_offset: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LinearTargets {
    loudness_lufs: f64,
    loudness_range_lu: f64,
    relaxed_loudness: bool,
    relaxed_range: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizeOutcome {
    Written,
    NoOp,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("FFmpeg initialization or processing failed: {0}")]
    Ffmpeg(#[from] ffmpeg::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no decodable audio stream was found in {0}")]
    NoAudio(PathBuf),
    #[error("the selected audio stream contains no decoded frames")]
    NoFrames,
    #[error("invalid normalization options: {0}")]
    InvalidOptions(String),
    #[error("invalid loudnorm analysis output: {0}")]
    InvalidLoudnorm(String),
    #[error("required FFmpeg filter `{0}` is unavailable")]
    MissingFilter(&'static str),
    #[error("required FFmpeg encoder `{0}` is unavailable")]
    MissingEncoder(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

struct InputAudio {
    context: format::context::Input,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    decoder: codec::decoder::Audio,
}

fn initialize() -> Result<()> {
    ffmpeg::init()?;
    ffmpeg::log::set_level(ffmpeg::log::Level::Warning);
    Ok(())
}

fn open_audio(path: &Path) -> Result<InputAudio> {
    let context = format::input(path)?;
    let stream = context
        .streams()
        .best(media::Type::Audio)
        .ok_or_else(|| Error::NoAudio(path.to_owned()))?;
    let stream_index = stream.index();
    let stream_time_base = stream.time_base();
    let mut decoder = codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .audio()?;
    decoder.set_parameters(stream.parameters())?;
    Ok(InputAudio {
        context,
        stream_index,
        stream_time_base,
        decoder,
    })
}

/// Opens the best audio stream and verifies that it can be decoded.
pub fn check(path: impl AsRef<Path>) -> Result<AudioInfo> {
    initialize()?;
    let path = path.as_ref();
    let mut input = open_audio(path)?;
    let info = AudioInfo {
        codec: input.decoder.id().name().to_owned(),
        sample_format: input.decoder.format().name().to_owned(),
        sample_rate: input.decoder.rate(),
        channels: input.decoder.channels(),
    };
    let mut frames = 0_u64;
    for (stream, packet) in input.context.packets() {
        if stream.index() != input.stream_index {
            continue;
        }
        input.decoder.send_packet(&packet)?;
        frames += drain_decoded(&mut input.decoder, |_| Ok(()))?;
    }
    input.decoder.send_eof()?;
    frames += drain_decoded(&mut input.decoder, |_| Ok(()))?;
    if frames == 0 {
        return Err(Error::NoFrames);
    }
    Ok(info)
}

fn drain_decoded(
    decoder: &mut codec::decoder::Audio,
    mut consume: impl FnMut(&mut frame::Audio) -> Result<()>,
) -> Result<u64> {
    let mut count = 0;
    loop {
        let mut decoded = frame::Audio::empty();
        match decoder.receive_frame(&mut decoded) {
            Ok(()) => {
                count += 1;
                consume(&mut decoded)?;
            }
            Err(error) if error == EAGAIN || error == ffmpeg::Error::Eof => return Ok(count),
            Err(error) => return Err(error.into()),
        }
    }
}

fn filter_by_name(name: &'static str) -> Result<filter::Filter> {
    filter::find(name).ok_or(Error::MissingFilter(name))
}

fn append_filter(
    graph: &mut filter::Graph,
    previous: &mut filter::Context,
    kind: &'static str,
    name: &str,
    args: &str,
) -> Result<filter::Context> {
    let mut next = graph.add(&filter_by_name(kind)?, name, args)?;
    previous.link(0, &mut next, 0);
    Ok(next)
}

fn graph_source(
    graph: &mut filter::Graph,
    decoder: &codec::decoder::Audio,
) -> Result<filter::Context> {
    let mut channel_layout = decoder.channel_layout();
    if channel_layout.is_empty() {
        channel_layout = ffmpeg::ChannelLayout::default(i32::from(decoder.channels()));
    }
    let args = format!(
        "time_base={}:sample_rate={}:sample_fmt={}:channel_layout=0x{:x}",
        decoder.time_base(),
        decoder.rate(),
        decoder.format().name(),
        channel_layout.bits()
    );
    Ok(graph.add(&filter_by_name("abuffer")?, "in", &args)?)
}

fn apply_offset(
    graph: &mut filter::Graph,
    mut previous: filter::Context,
    decoder_rate: u32,
    options: &NormalizeOptions,
) -> Result<filter::Context> {
    if options.offset_seconds.abs() < options.offset_tolerance_seconds {
        return Ok(previous);
    }
    if options.offset_seconds > 0.0 {
        let delay_ms = (options.offset_seconds * 1_000.0).round();
        if !(0.0..=(i64::MAX as f64)).contains(&delay_ms) {
            return Err(Error::InvalidOptions("positive offset is too large".into()));
        }
        append_filter(
            graph,
            &mut previous,
            "adelay",
            "offset_delay",
            &format!("delays={}:all=1", delay_ms as i64),
        )
    } else {
        let start_sample = (-options.offset_seconds * f64::from(decoder_rate)).round();
        if !(0.0..=(i64::MAX as f64)).contains(&start_sample) {
            return Err(Error::InvalidOptions("negative offset is too large".into()));
        }
        let mut trimmed = append_filter(
            graph,
            &mut previous,
            "atrim",
            "offset_trim",
            &format!("start_sample={}", start_sample as i64),
        )?;
        append_filter(
            graph,
            &mut trimmed,
            "asetpts",
            "offset_pts",
            "expr=PTS-STARTPTS",
        )
    }
}

fn linear_targets(stats: LoudnessStats, options: &NormalizeOptions) -> LinearTargets {
    let max_linear_lufs = stats.input_i + (options.true_peak_dbtp - stats.input_tp);
    let loudness_lufs = options.loudness_lufs.min(max_linear_lufs);
    let loudness_range_lu = options.loudness_range_lu.max(stats.input_lra);
    LinearTargets {
        loudness_lufs,
        loudness_range_lu,
        relaxed_loudness: loudness_lufs + f64::EPSILON < options.loudness_lufs,
        relaxed_range: loudness_range_lu > options.loudness_range_lu + f64::EPSILON,
    }
}

fn loudnorm_args(
    options: &NormalizeOptions,
    targets: LinearTargets,
    stats: Option<LoudnessStats>,
) -> String {
    let mut args = format!(
        "I={}:LRA={}:TP={}:linear=true",
        targets.loudness_lufs, targets.loudness_range_lu, options.true_peak_dbtp
    );
    if let Some(stats) = stats {
        // offset is overwritten by FFmpeg in linear mode (gain = target_i - measured_i);
        // still pass the analysis suggestion for filter versions that consume it.
        args.push_str(&format!(
            ":measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}",
            stats.input_i, stats.input_tp, stats.input_lra, stats.input_thresh, stats.target_offset
        ));
    }
    args
}

fn run_filter_to_end(input: &mut InputAudio, graph: &mut filter::Graph) -> Result<u64> {
    let mut frames = 0;
    for (stream, mut packet) in input.context.packets() {
        if stream.index() != input.stream_index {
            continue;
        }
        packet.rescale_ts(stream.time_base(), input.decoder.time_base());
        input.decoder.send_packet(&packet)?;
        frames += drain_decoded(&mut input.decoder, |decoded| {
            let timestamp = decoded.timestamp();
            decoded.set_pts(timestamp);
            graph
                .get("in")
                .ok_or(ffmpeg::Error::InvalidData)?
                .source()
                .add(decoded)?;
            drain_filter(graph, |_| Ok(()))?;
            Ok(())
        })?;
    }
    input.decoder.send_eof()?;
    frames += drain_decoded(&mut input.decoder, |decoded| {
        let timestamp = decoded.timestamp();
        decoded.set_pts(timestamp);
        graph
            .get("in")
            .ok_or(ffmpeg::Error::InvalidData)?
            .source()
            .add(decoded)?;
        drain_filter(graph, |_| Ok(()))?;
        Ok(())
    })?;
    graph
        .get("in")
        .ok_or(ffmpeg::Error::InvalidData)?
        .source()
        .flush()?;
    drain_filter(graph, |_| Ok(()))?;
    Ok(frames)
}

fn drain_filter(
    graph: &mut filter::Graph,
    mut consume: impl FnMut(&mut frame::Audio) -> Result<()>,
) -> Result<u64> {
    let mut count = 0;
    loop {
        let mut filtered = frame::Audio::empty();
        let result = graph
            .get("out")
            .ok_or(ffmpeg::Error::InvalidData)?
            .sink()
            .frame(&mut filtered);
        match result {
            Ok(()) => {
                count += 1;
                consume(&mut filtered)?;
            }
            Err(error) if error == EAGAIN || error == ffmpeg::Error::Eof => return Ok(count),
            Err(error) => return Err(error.into()),
        }
    }
}

struct StatsPath(PathBuf);

impl StatsPath {
    fn create() -> Result<Self> {
        let file = tempfile::Builder::new()
            .prefix("mua-loudnorm-")
            .suffix(".json")
            .tempfile()?;
        let path = file
            .into_temp_path()
            .keep()
            .map_err(|error| Error::Io(error.error))?;
        fs::remove_file(&path)?;
        Ok(Self(path))
    }
}

impl Drop for StatsPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn analyze_loudness(path: &Path, options: &NormalizeOptions) -> Result<(AudioInfo, LoudnessStats)> {
    let stats_path = StatsPath::create()?;
    let mut input = open_audio(path)?;
    let info = AudioInfo {
        codec: input.decoder.id().name().to_owned(),
        sample_format: input.decoder.format().name().to_owned(),
        sample_rate: input.decoder.rate(),
        channels: input.decoder.channels(),
    };
    let mut graph = filter::Graph::new();
    let source = graph_source(&mut graph, &input.decoder)?;
    let mut previous = apply_offset(&mut graph, source, input.decoder.rate(), options)?;
    let stats_file = stats_path
        .0
        .to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "\\'");
    let mut loudnorm = append_filter(
        &mut graph,
        &mut previous,
        "loudnorm",
        "loudnorm_probe",
        &format!(
            "{}:print_format=json:stats_file='{}'",
            loudnorm_args(options, LinearTargets {
                loudness_lufs: options.loudness_lufs,
                loudness_range_lu: options.loudness_range_lu,
                relaxed_loudness: false,
                relaxed_range: false,
            }, None),
            stats_file
        ),
    )?;
    let mut sink = graph.add(&filter_by_name("abuffersink")?, "out", "")?;
    loudnorm.link(0, &mut sink, 0);
    graph.validate()?;
    if run_filter_to_end(&mut input, &mut graph)? == 0 {
        return Err(Error::NoFrames);
    }
    drop(graph);
    let json = fs::read_to_string(&stats_path.0)?;
    let stats = parse_loudnorm_stats(&json)?;
    info!(
        input_i = stats.input_i,
        input_tp = stats.input_tp,
        input_lra = stats.input_lra,
        input_thresh = stats.input_thresh,
        target_offset = stats.target_offset,
        "completed loudnorm analysis pass"
    );
    Ok((info, stats))
}

fn parse_json_number(json: &str, key: &'static str) -> Result<f64> {
    let needle = format!("\"{key}\"");
    let start = json
        .find(&needle)
        .ok_or_else(|| Error::InvalidLoudnorm(format!("missing `{key}`")))?;
    let rest = &json[start + needle.len()..];
    let colon = rest
        .find(':')
        .ok_or_else(|| Error::InvalidLoudnorm(format!("malformed `{key}`")))?;
    let value = rest[colon + 1..].trim_start().trim_start_matches('"');
    let end = value
        .find(|character: char| {
            !(character.is_ascii_digit() || matches!(character, '-' | '+' | '.' | 'e' | 'E'))
        })
        .unwrap_or(value.len());
    let parsed = value[..end]
        .parse::<f64>()
        .map_err(|_| Error::InvalidLoudnorm(format!("invalid `{key}`")))?;
    if !parsed.is_finite() {
        return Err(Error::InvalidLoudnorm(format!("non-finite `{key}`")));
    }
    Ok(parsed)
}

fn parse_loudnorm_stats(json: &str) -> Result<LoudnessStats> {
    Ok(LoudnessStats {
        input_i: parse_json_number(json, "input_i")?,
        input_tp: parse_json_number(json, "input_tp")?,
        input_lra: parse_json_number(json, "input_lra")?,
        input_thresh: parse_json_number(json, "input_thresh")?,
        target_offset: parse_json_number(json, "target_offset")?,
    })
}

fn is_noop(info: &AudioInfo, stats: LoudnessStats, options: &NormalizeOptions) -> bool {
    let targets = linear_targets(stats, options);
    info.codec == options.sample_format.codec().name()
        && info.sample_rate == options.sample_rate
        && info.sample_format == options.sample_format.name()
        && info.channels == 2
        && (stats.input_i - targets.loudness_lufs).abs() < options.gain_tolerance_db
        && stats.input_tp <= options.true_peak_dbtp + options.true_peak_tolerance_db
        && options.offset_seconds.abs() < options.offset_tolerance_seconds
}

fn needs_loudness_adjustment(
    stats: LoudnessStats,
    options: &NormalizeOptions,
    targets: LinearTargets,
) -> bool {
    (stats.input_i - targets.loudness_lufs).abs() >= options.gain_tolerance_db
        || stats.input_tp > options.true_peak_dbtp + options.true_peak_tolerance_db
}

fn transcode(
    source: &Path,
    output: &Path,
    options: &NormalizeOptions,
    stats: LoudnessStats,
    targets: LinearTargets,
    apply_loudnorm: bool,
) -> Result<()> {
    let mut input = open_audio(source)?;
    let mut output_context = format::output_as(output, "wav")?;
    let codec = codec::encoder::find(options.sample_format.codec())
        .ok_or(Error::MissingEncoder(options.sample_format.name()))?;
    let global_header = output_context
        .format()
        .flags()
        .contains(format::Flags::GLOBAL_HEADER);
    let mut encoder = codec::context::Context::new_with_codec(codec)
        .encoder()
        .audio()?;
    encoder.set_rate(options.sample_rate as i32);
    encoder.set_format(options.sample_format.ffmpeg());
    encoder.set_channel_layout(ffmpeg::ChannelLayout::STEREO);
    encoder.set_time_base((1, options.sample_rate as i32));
    if global_header {
        encoder.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    let mut encoder = encoder.open_as(codec)?;
    let output_stream_index = {
        let mut output_stream = output_context.add_stream(codec)?;
        output_stream.set_time_base((1, options.sample_rate as i32));
        output_stream.set_parameters(&encoder);
        output_stream.index()
    };

    let mut graph = filter::Graph::new();
    let source_filter = graph_source(&mut graph, &input.decoder)?;
    let mut previous = apply_offset(&mut graph, source_filter, input.decoder.rate(), options)?;
    if apply_loudnorm {
        previous = append_filter(
            &mut graph,
            &mut previous,
            "loudnorm",
            "loudnorm_apply",
            &loudnorm_args(options, targets, Some(stats)),
        )?;
    }
    let mut converted = append_filter(
        &mut graph,
        &mut previous,
        "aformat",
        "target_format",
        &format!(
            "sample_fmts={}:sample_rates={}:channel_layouts=stereo",
            options.sample_format.name(),
            options.sample_rate
        ),
    )?;
    let mut sink = graph.add(&filter_by_name("abuffersink")?, "out", "")?;
    converted.link(0, &mut sink, 0);
    graph.validate()?;
    let sink_time_base = graph
        .get("out")
        .ok_or(ffmpeg::Error::InvalidData)?
        .sink()
        .time_base();

    output_context.write_header()?;
    let output_time_base = output_context
        .stream(output_stream_index)
        .ok_or(ffmpeg::Error::InvalidData)?
        .time_base();

    let mut process_filtered = |filtered: &mut frame::Audio| -> Result<()> {
        if let Some(pts) = filtered.pts() {
            filtered.set_pts(Some(pts.rescale(sink_time_base, encoder.time_base())));
        }
        encoder.send_frame(filtered)?;
        drain_encoded(
            &mut encoder,
            &mut output_context,
            output_stream_index,
            output_time_base,
        )
    };

    for (stream, mut packet) in input.context.packets() {
        if stream.index() != input.stream_index {
            continue;
        }
        packet.rescale_ts(input.stream_time_base, input.decoder.time_base());
        input.decoder.send_packet(&packet)?;
        drain_decoded(&mut input.decoder, |decoded| {
            let timestamp = decoded.timestamp();
            decoded.set_pts(timestamp);
            graph
                .get("in")
                .ok_or(ffmpeg::Error::InvalidData)?
                .source()
                .add(decoded)?;
            drain_filter(&mut graph, &mut process_filtered)?;
            Ok(())
        })?;
    }
    input.decoder.send_eof()?;
    drain_decoded(&mut input.decoder, |decoded| {
        let timestamp = decoded.timestamp();
        decoded.set_pts(timestamp);
        graph
            .get("in")
            .ok_or(ffmpeg::Error::InvalidData)?
            .source()
            .add(decoded)?;
        drain_filter(&mut graph, &mut process_filtered)?;
        Ok(())
    })?;
    graph
        .get("in")
        .ok_or(ffmpeg::Error::InvalidData)?
        .source()
        .flush()?;
    drain_filter(&mut graph, &mut process_filtered)?;
    encoder.send_eof()?;
    drain_encoded(
        &mut encoder,
        &mut output_context,
        output_stream_index,
        output_time_base,
    )?;
    output_context.write_trailer()?;
    Ok(())
}

fn drain_encoded(
    encoder: &mut codec::encoder::Audio,
    output: &mut format::context::Output,
    stream_index: usize,
    output_time_base: ffmpeg::Rational,
) -> Result<()> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(stream_index);
                packet.rescale_ts(encoder.time_base(), output_time_base);
                packet.write_interleaved(output)?;
            }
            Err(error) if error == EAGAIN || error == ffmpeg::Error::Eof => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
}

/// Normalizes `source` into a stereo PCM WAV. The destination is published
/// atomically only after FFmpeg finishes the complete file.
pub fn normalize(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    options: &NormalizeOptions,
) -> Result<NormalizeOutcome> {
    initialize()?;
    options.validate()?;
    let source = source.as_ref();
    let destination = destination.as_ref();
    let (info, stats) = analyze_loudness(source, options)?;
    let targets = linear_targets(stats, options);
    if targets.relaxed_loudness || targets.relaxed_range {
        info!(
            requested_i = options.loudness_lufs,
            effective_i = targets.loudness_lufs,
            requested_lra = options.loudness_range_lu,
            effective_lra = targets.loudness_range_lu,
            input_tp = stats.input_tp,
            target_tp = options.true_peak_dbtp,
            "relaxed loudnorm targets to keep linear gain (avoid dynamic pumping)"
        );
    }
    if is_noop(&info, stats, options) {
        info!("source already satisfies every normalization target");
        return Ok(NormalizeOutcome::NoOp);
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = tempfile::Builder::new()
        .prefix(".mua-wav-")
        .suffix(".wav")
        .tempfile_in(parent)?;
    let temporary_path = temporary.path().to_owned();
    drop(temporary);
    let apply_loudnorm = needs_loudness_adjustment(stats, options, targets);
    let result = transcode(
        source,
        &temporary_path,
        options,
        stats,
        targets,
        apply_loudnorm,
    );
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(&temporary_path, destination)?;
    Ok(NormalizeOutcome::Written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn pcm_wav(seconds: u32) -> Vec<u8> {
        let frames = 48_000 * seconds;
        let data_size = frames * 2;
        let mut wav = Vec::with_capacity(44 + data_size as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&48_000_u32.to_le_bytes());
        wav.extend_from_slice(&96_000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for sample_index in 0..frames {
            let phase = f64::from(sample_index) * 440.0 * std::f64::consts::TAU / 48_000.0;
            let sample = (phase.sin() * 3_000.0) as i16;
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    #[test]
    fn parses_sample_format_aliases() {
        assert_eq!("s16".parse::<SampleFormat>().unwrap(), SampleFormat::S16);
        assert_eq!("f32".parse::<SampleFormat>().unwrap(), SampleFormat::F32);
        assert!("s24".parse::<SampleFormat>().is_err());
    }

    #[test]
    fn parses_quoted_loudnorm_json_numbers() {
        let stats = parse_loudnorm_stats(
            r#"{
                "input_i": "-10.20",
                "input_tp": "-0.10",
                "input_lra": "1.00",
                "input_thresh": "-20.30",
                "target_offset": "0.02"
            }"#,
        )
        .unwrap();
        assert_eq!(stats.input_i, -10.2);
        assert_eq!(stats.target_offset, 0.02);
    }

    #[test]
    fn relaxes_integrated_target_when_true_peak_blocks_gain() {
        let stats = LoudnessStats {
            input_i: -10.83,
            input_tp: 0.09,
            input_lra: 8.0,
            input_thresh: -21.0,
            target_offset: 1.2,
        };
        let options = NormalizeOptions::default();
        let targets = linear_targets(stats, &options);
        assert!(targets.relaxed_loudness);
        assert!(!targets.relaxed_range);
        assert!((targets.loudness_lufs - (-10.92)).abs() < 1e-9);
        assert_eq!(targets.loudness_range_lu, 11.0);
        assert!(!needs_loudness_adjustment(stats, &options, targets));
    }

    #[test]
    fn raises_lra_floor_instead_of_forcing_dynamic_compression() {
        let stats = LoudnessStats {
            input_i: -14.0,
            input_tp: -6.0,
            input_lra: 18.0,
            input_thresh: -24.0,
            target_offset: 0.0,
        };
        let options = NormalizeOptions::default();
        let targets = linear_targets(stats, &options);
        assert!(!targets.relaxed_loudness);
        assert!(targets.relaxed_range);
        assert_eq!(targets.loudness_lufs, -8.25);
        assert_eq!(targets.loudness_range_lu, 18.0);
        assert!(needs_loudness_adjustment(stats, &options, targets));
    }

    #[test]
    fn keeps_requested_targets_when_linear_gain_fits() {
        let stats = LoudnessStats {
            input_i: -20.0,
            input_tp: -15.0,
            input_lra: 7.0,
            input_thresh: -30.0,
            target_offset: 0.0,
        };
        let options = NormalizeOptions::default();
        let targets = linear_targets(stats, &options);
        assert!(!targets.relaxed_loudness);
        assert!(!targets.relaxed_range);
        assert_eq!(targets.loudness_lufs, -8.25);
        assert_eq!(targets.loudness_range_lu, 11.0);
    }

    #[test]
    fn rejects_non_finite_options() {
        let options = NormalizeOptions {
            offset_seconds: f64::NAN,
            ..NormalizeOptions::default()
        };
        assert!(options.validate().is_err());
    }

    #[test]
    fn validates_and_normalizes_real_pcm_audio() {
        let directory = tempdir().expect("temporary directory should be created");
        let input = directory.path().join("input.wav");
        let output = directory.path().join("output.wav");
        let trimmed = directory.path().join("trimmed.wav");
        fs::write(&input, pcm_wav(1)).expect("WAV fixture should be written");

        let source_info = check(&input).expect("PCM WAV should decode");
        assert_eq!(source_info.channels, 1);
        let mut options = NormalizeOptions {
            offset_seconds: 0.01,
            sample_format: SampleFormat::S32,
            sample_rate: 44_100,
            loudness_lufs: -12.0,
            true_peak_dbtp: -1.0,
            ..NormalizeOptions::default()
        };
        assert_eq!(
            normalize(&input, &output, &options).expect("normalization should succeed"),
            NormalizeOutcome::Written
        );
        let output_info = check(&output).expect("normalized WAV should decode");
        assert_eq!(output_info.channels, 2);
        assert_eq!(output_info.sample_rate, 44_100);
        assert_eq!(output_info.sample_format, "s32");

        let no_op_output = directory.path().join("no-op.wav");
        options.offset_seconds = 0.0;
        assert_eq!(
            normalize(&output, &no_op_output, &options)
                .expect("already-normalized input should be inspected"),
            NormalizeOutcome::NoOp
        );
        assert!(!no_op_output.exists());

        options.offset_seconds = -0.01;
        assert_eq!(
            normalize(&input, &trimmed, &options).expect("trimming should succeed"),
            NormalizeOutcome::Written
        );

        fs::write(directory.path().join("invalid.wav"), b"not audio")
            .expect("invalid fixture should be written");
        assert!(check(directory.path().join("invalid.wav")).is_err());
    }
}
