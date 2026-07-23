use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use mua_wav::{NormalizeOptions, NormalizeOutcome, SampleFormat};
use tracing::error;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "mua_wav",
    version,
    about = "Validate and normalize audio with FFmpeg"
)]
struct Cli {
    #[arg(long, global = true, default_value = "info")]
    log_level: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate the best audio stream by decoding it completely.
    Check(Source),
    /// Normalize to a stereo PCM WAV using measured two-pass loudnorm.
    Normalize(Normalize),
}

#[derive(Debug, Args)]
struct Source {
    #[arg(short = 's', long = "source")]
    input: PathBuf,
}

#[derive(Debug, Args)]
struct Normalize {
    #[arg(short = 's', long = "source")]
    input: PathBuf,
    #[arg(short = 'd', long = "destination")]
    output: PathBuf,
    #[arg(short = 'o', long = "offset", default_value_t = 0.0)]
    offset_seconds: f64,
    #[arg(long, default_value = "s16")]
    sample_format: SampleFormat,
    #[arg(long, default_value_t = 48_000)]
    sample_rate: u32,
    #[arg(long = "lufs", default_value_t = -8.25)]
    loudness_lufs: f64,
    #[arg(long = "lu", default_value_t = 11.0)]
    loudness_range_lu: f64,
    #[arg(long = "dbtp", default_value_t = 0.0)]
    true_peak_dbtp: f64,
    #[arg(long, default_value_t = 0.5)]
    true_peak_tolerance: f64,
    #[arg(long = "lu-tolerance", default_value_t = 0.1)]
    loudness_range_tolerance: f64,
    #[arg(long, default_value_t = 0.2)]
    gain_tolerance: f64,
    #[arg(long, default_value_t = 0.000_1)]
    offset_tolerance: f64,
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { 64 } else { 0 };
            let _ = error.print();
            std::process::exit(code);
        }
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&cli.log_level).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let result = match cli.command {
        Command::Check(args) => mua_wav::check(args.input).map(|_| NormalizeOutcome::Written),
        Command::Normalize(args) => mua_wav::normalize(
            args.input,
            args.output,
            &NormalizeOptions {
                offset_seconds: args.offset_seconds,
                sample_format: args.sample_format,
                sample_rate: args.sample_rate,
                loudness_lufs: args.loudness_lufs,
                loudness_range_lu: args.loudness_range_lu,
                true_peak_dbtp: args.true_peak_dbtp,
                true_peak_tolerance_db: args.true_peak_tolerance,
                loudness_range_tolerance_lu: args.loudness_range_tolerance,
                gain_tolerance_db: args.gain_tolerance,
                offset_tolerance_seconds: args.offset_tolerance,
            },
        ),
    };
    match result {
        Ok(NormalizeOutcome::Written) => {}
        Ok(NormalizeOutcome::NoOp) => std::process::exit(2),
        Err(problem) => {
            error!(error = %problem, "operation failed");
            std::process::exit(1);
        }
    }
}
