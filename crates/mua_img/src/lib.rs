//! Image conversion and AFB/DDS container operations.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use dds::{
    ColorFormat, CompressionQuality, Encoder, Format, ImageView, ImageViewMut, Size,
    header::{Dx9Header, FourCC, Header},
};
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage, imageops::FilterType};
use thiserror::Error;

const DDS_MAGIC: &[u8; 4] = b"DDS ";
const DDS_FOOTER: &[u8; 4] = b"POF0";

/// Errors returned by image and AFB operations.
#[derive(Debug, Error)]
pub enum Error {
    /// A filesystem operation failed.
    #[error("{operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Raster decoding or transformation failed.
    #[error("image operation failed for {path}: {source}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    /// DDS encoding failed.
    #[error("DDS encoding failed: {0}")]
    Dds(String),
    /// The source contains no DDS chunks.
    #[error("no DDS chunks found in {0}")]
    NoDdsChunks(PathBuf),
    /// A stage template does not have the required background and effect chunks.
    #[error("stage template requires at least two DDS chunks, found {0}")]
    InsufficientStageChunks(usize),
    /// A computed chunk range is malformed.
    #[error("invalid chunk range [{start}, {end}) for data length {length}")]
    InvalidChunkRange {
        start: usize,
        end: usize,
        length: usize,
    },
}

/// Convenient result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

fn io_error(operation: &'static str, path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|source| io_error("read", path, source))
}

fn write(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data).map_err(|source| io_error("write", path, source))
}

fn load_oriented(path: &Path) -> Result<DynamicImage> {
    let reader = ImageReader::open(path)
        .map_err(|source| io_error("open", path, source))?
        .with_guessed_format()
        .map_err(|source| io_error("detect image format", path, source))?;
    let mut decoder = reader.into_decoder().map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;
    let orientation = decoder.orientation().map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn resized_rgba(path: &Path, width: u32, height: u32) -> Result<RgbaImage> {
    Ok(load_oriented(path)?
        .resize_exact(width, height, FilterType::Lanczos3)
        .to_rgba8())
}

fn encode_dds(image: &RgbaImage, format: Format) -> Result<Vec<u8>> {
    let width = image.width();
    let height = image.height();
    // CHUNITHM's existing assets use legacy DXT1/DXT5 FourCC headers. The
    // equivalent DX10 headers are not accepted by every older texture loader.
    let four_cc = FourCC::try_from(format)
        .map_err(|()| Error::Dds(format!("{format:?} has no legacy FourCC representation")))?;
    let header = Header::Dx9(Dx9Header::new_image(width, height, four_cc.into()));
    let mut output = Vec::new();
    let mut encoder = Encoder::new(&mut output, format, &header)
        .map_err(|error| Error::Dds(error.to_string()))?;
    encoder.encoding.quality = CompressionQuality::High;
    let view = ImageView::new(
        image.as_raw(),
        Size::new(width, height),
        ColorFormat::RGBA_U8,
    )
    .ok_or_else(|| Error::Dds("invalid RGBA image view".to_owned()))?;
    encoder
        .write_surface(view)
        .map_err(|error| Error::Dds(error.to_string()))?;
    encoder
        .finish()
        .map_err(|error| Error::Dds(error.to_string()))?;
    Ok(output)
}

fn find_all(data: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || data.len() < needle.len() {
        return Vec::new();
    }
    data.windows(needle.len())
        .enumerate()
        .filter_map(|(position, candidate)| (candidate == needle).then_some(position))
        .collect()
}

/// Locate embedded DDS byte ranges in an AFB-like container.
pub fn locate_dds_chunks(data: &[u8]) -> Vec<std::ops::Range<usize>> {
    let headers = find_all(data, DDS_MAGIC);
    let footers = find_all(data, DDS_FOOTER);
    headers
        .iter()
        .enumerate()
        .map(|(index, &start)| {
            let next_header = headers.get(index + 1).copied().unwrap_or(data.len());
            let footer = footers
                .iter()
                .copied()
                .find(|&position| position >= start + DDS_MAGIC.len())
                .unwrap_or(data.len());
            start..footer.min(next_header)
        })
        .collect()
}

fn validate_chunks(data: &[u8], chunks: &[std::ops::Range<usize>]) -> Result<()> {
    let mut cursor = 0;
    for chunk in chunks {
        if chunk.start < cursor || chunk.start > chunk.end || chunk.end > data.len() {
            return Err(Error::InvalidChunkRange {
                start: chunk.start,
                end: chunk.end,
                length: data.len(),
            });
        }
        cursor = chunk.end;
    }
    Ok(())
}

fn replace_chunks(
    data: &[u8],
    chunks: &[std::ops::Range<usize>],
    replacements: &[&[u8]],
) -> Result<Vec<u8>> {
    validate_chunks(data, chunks)?;
    let mut capacity = data.len();
    for (chunk, replacement) in chunks.iter().zip(replacements) {
        capacity = capacity - chunk.len() + replacement.len();
    }
    let mut output = Vec::with_capacity(capacity);
    let mut cursor = 0;
    for (index, chunk) in chunks.iter().enumerate() {
        output.extend_from_slice(&data[cursor..chunk.start]);
        if let Some(replacement) = replacements.get(index) {
            output.extend_from_slice(replacement);
        } else {
            output.extend_from_slice(&data[chunk.clone()]);
        }
        cursor = chunk.end;
    }
    output.extend_from_slice(&data[cursor..]);
    Ok(output)
}

fn effect_atlas(paths: &[Option<PathBuf>; 4]) -> Result<RgbaImage> {
    const TILE: u32 = 256;
    let mut atlas = RgbaImage::new(TILE * 2, TILE * 2);
    for (index, path) in paths.iter().enumerate() {
        let tile = match path {
            Some(path) => resized_rgba(path, TILE, TILE)?,
            None => RgbaImage::new(TILE, TILE),
        };
        let offset_x = u32::try_from(index % 2).unwrap_or(0) * TILE;
        let offset_y = u32::try_from(index / 2).unwrap_or(0) * TILE;
        for (x, y, pixel) in tile.enumerate_pixels() {
            atlas.put_pixel(offset_x + x, offset_y + y, *pixel);
        }
    }
    Ok(atlas)
}

/// Check that a raster file has a supported decoder and non-zero dimensions.
pub fn check(path: &Path) -> Result<()> {
    let reader = ImageReader::open(path)
        .map_err(|source| io_error("open", path, source))?
        .with_guessed_format()
        .map_err(|source| io_error("detect image format", path, source))?;
    let decoder = reader.into_decoder().map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(Error::Image {
            path: path.to_path_buf(),
            source: image::ImageError::Limits(image::error::LimitError::from_kind(
                image::error::LimitErrorKind::DimensionError,
            )),
        });
    }
    Ok(())
}

/// Convert a raster image to a 300x300 BC1 DDS jacket.
pub fn convert_jacket(source: &Path, destination: &Path) -> Result<()> {
    let image = resized_rgba(source, 300, 300)?;
    write(destination, &encode_dds(&image, Format::BC1_UNORM)?)
}

/// Convert and inject a stage background and effect atlas into an AFB template.
pub fn convert_stage(
    background: &Path,
    destination: &Path,
    effects: &[Option<PathBuf>; 4],
    template: Option<&Path>,
    notes_field: Option<&Path>,
) -> Result<()> {
    let template_data = match template {
        Some(path) => read(path)?,
        None => mua_assets::ST_DUMMY_AFB.to_vec(),
    };
    let chunks = locate_dds_chunks(&template_data);
    if chunks.len() < 2 {
        return Err(Error::InsufficientStageChunks(chunks.len()));
    }
    validate_chunks(&template_data, &chunks)?;

    let background = encode_dds(&resized_rgba(background, 1920, 1080)?, Format::BC1_UNORM)?;
    let effects = encode_dds(&effect_atlas(effects)?, Format::BC3_UNORM)?;
    let replaced = replace_chunks(&template_data, &chunks, &[&background, &effects])?;
    write(destination, &replaced)?;
    if let Some(path) = notes_field {
        write(path, mua_assets::NF_DUMMY_AFB)?;
    }
    Ok(())
}

/// Extract every embedded DDS chunk from a container.
pub fn extract_dds(source: &Path, destination: &Path) -> Result<Vec<PathBuf>> {
    let data = read(source)?;
    let chunks = locate_dds_chunks(&data);
    if chunks.is_empty() {
        return Err(Error::NoDdsChunks(source.to_path_buf()));
    }
    validate_chunks(&data, &chunks)?;
    fs::create_dir_all(destination)
        .map_err(|source| io_error("create directory", destination, source))?;

    let stem = source
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| std::ffi::OsStr::new("chunk"));
    let mut paths = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        let filename = format!("{}_{:04}.dds", stem.to_string_lossy(), index + 1);
        let path = destination.join(filename);
        write(&path, &data[chunk.clone()])?;
        paths.push(path);
    }
    Ok(paths)
}

/// Decode enough DDS metadata to support integration tests and callers that need validation.
pub fn inspect_dds(data: &[u8]) -> Result<(Format, u32, u32)> {
    let decoder =
        dds::Decoder::new(Cursor::new(data)).map_err(|error| Error::Dds(error.to_string()))?;
    let size = decoder.main_size();
    Ok((decoder.format(), size.width, size.height))
}

/// Decode the main DDS surface to a PNG image.
pub fn decode_dds(source: &Path, destination: &Path) -> Result<()> {
    let data = read(source)?;
    let mut decoder =
        dds::Decoder::new(Cursor::new(data)).map_err(|error| Error::Dds(error.to_string()))?;
    let size = decoder.main_size();
    let mut rgba = vec![0_u8; size.pixels() as usize * 4];
    let view = ImageViewMut::new(&mut rgba, size, ColorFormat::RGBA_U8)
        .ok_or_else(|| Error::Dds("invalid output image view".to_owned()))?;
    decoder
        .read_surface(view)
        .map_err(|error| Error::Dds(error.to_string()))?;
    let image = RgbaImage::from_raw(size.width, size.height, rgba)
        .ok_or_else(|| Error::Dds("invalid decoded RGBA image".to_owned()))?;
    image
        .save_with_format(destination, image::ImageFormat::Png)
        .map_err(|source| Error::Image {
            path: destination.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, ImageFormat, Rgba, RgbaImage};
    use tempfile::tempdir;

    fn fixture(path: &Path, width: u32, height: u32, color: Rgba<u8>) {
        RgbaImage::from_pixel(width, height, color)
            .save(path)
            .expect("fixture image should save");
    }

    #[test]
    fn jacket_is_bc1_at_expected_size() {
        let dir = tempdir().expect("temporary directory should be created");
        let source = dir.path().join("source.png");
        let output = dir.path().join("jacket.dds");
        fixture(&source, 20, 10, Rgba([10, 20, 30, 255]));

        convert_jacket(&source, &output).expect("jacket conversion should succeed");
        let bytes = fs::read(output).expect("DDS should be readable");
        let (format, width, height) = inspect_dds(&bytes).expect("DDS should parse");
        assert_eq!(format, Format::BC1_UNORM);
        assert_eq!((width, height), (300, 300));
        assert_eq!(&bytes[84..88], b"DXT1");
        assert_eq!(bytes.len(), 45_128);
    }

    #[test]
    fn dds_decodes_to_png() {
        let dir = tempdir().expect("temporary directory should be created");
        let source = dir.path().join("source.png");
        let dds = dir.path().join("jacket.dds");
        let png = dir.path().join("decoded.png");
        fixture(&source, 20, 10, Rgba([10, 20, 30, 255]));
        convert_jacket(&source, &dds).expect("DDS should encode");

        decode_dds(&dds, &png).expect("DDS should decode");

        let decoded = image::open(png).expect("PNG should open");
        assert_eq!(decoded.dimensions(), (300, 300));
    }

    #[test]
    fn stage_template_has_embedded_dds_chunks() {
        let chunks = locate_dds_chunks(mua_assets::ST_DUMMY_AFB);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn stage_replaces_two_chunks_and_preserves_container_data() {
        let dir = tempdir().expect("temporary directory should be created");
        let background = dir.path().join("background.png");
        let effect = dir.path().join("effect.png");
        fixture(&background, 32, 18, Rgba([1, 2, 3, 255]));
        fixture(&effect, 8, 8, Rgba([4, 5, 6, 128]));
        let template = dir.path().join("stage.afb");
        fs::write(
            &template,
            b"prefixDDS placeholder onePOF0middleDDS placeholder twoPOF0suffix",
        )
        .expect("template should save");
        let output = dir.path().join("converted.afb");

        convert_stage(
            &background,
            &output,
            &[Some(effect), None, None, None],
            Some(&template),
            None,
        )
        .expect("stage conversion should succeed");
        let data = fs::read(output).expect("converted stage should read");
        assert!(data.starts_with(b"prefix"));
        assert!(data.ends_with(b"suffix"));
        let chunks = locate_dds_chunks(&data);
        assert_eq!(chunks.len(), 2);
        let first = inspect_dds(&data[chunks[0].clone()]).expect("background DDS should parse");
        let second = inspect_dds(&data[chunks[1].clone()]).expect("effect DDS should parse");
        assert_eq!(first, (Format::BC1_UNORM, 1920, 1080));
        assert_eq!(second, (Format::BC3_UNORM, 512, 512));
        assert_eq!(&data[chunks[0].start + 84..chunks[0].start + 88], b"DXT1");
        assert_eq!(&data[chunks[1].start + 84..chunks[1].start + 88], b"DXT5");
    }

    #[test]
    fn extraction_uses_sequential_names() {
        let dir = tempdir().expect("temporary directory should be created");
        let source = dir.path().join("sample.afb");
        fs::write(&source, b"DDS firstPOF0gapDDS secondPOF0").expect("fixture should save");
        let output = dir.path().join("out");

        let paths = extract_dds(&source, &output).expect("extraction should succeed");
        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths[0].file_name().and_then(|name| name.to_str()),
            Some("sample_0001.dds")
        );
        assert_eq!(
            paths[1].file_name().and_then(|name| name.to_str()),
            Some("sample_0002.dds")
        );
    }

    #[test]
    fn invalid_image_is_rejected() {
        let dir = tempdir().expect("temporary directory should be created");
        let source = dir.path().join("invalid.png");
        fs::write(&source, b"not an image").expect("fixture should save");
        assert!(check(&source).is_err());
    }

    #[test]
    fn content_sniffing_does_not_depend_on_extension() {
        let dir = tempdir().expect("temporary directory should be created");
        let source = dir.path().join("image.unknown");
        RgbaImage::from_pixel(2, 3, Rgba([1, 2, 3, 255]))
            .save_with_format(&source, ImageFormat::Png)
            .expect("PNG fixture should save");
        check(&source).expect("image content should be detected");
    }

    #[test]
    fn exif_orientation_is_applied_before_resize() {
        let directory = tempdir().expect("temporary directory should be created");
        let source = directory.path().join("oriented.jpg");
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 3, Rgba([10, 20, 30, 255])));
        let mut jpeg = Cursor::new(Vec::new());
        image
            .write_to(&mut jpeg, ImageFormat::Jpeg)
            .expect("JPEG should encode");
        let jpeg = jpeg.into_inner();

        // EXIF/TIFF IFD0 with Orientation=6 (rotate 90 degrees clockwise).
        let mut exif = Vec::from(
            &b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0"[..],
        );
        let segment_length = u16::try_from(exif.len() + 2).expect("EXIF segment should fit");
        let mut oriented = Vec::with_capacity(jpeg.len() + exif.len() + 4);
        oriented.extend_from_slice(&jpeg[..2]);
        oriented.extend_from_slice(&[0xff, 0xe1]);
        oriented.extend_from_slice(&segment_length.to_be_bytes());
        oriented.append(&mut exif);
        oriented.extend_from_slice(&jpeg[2..]);
        fs::write(&source, oriented).expect("oriented JPEG should be written");

        assert_eq!(
            load_oriented(&source)
                .expect("oriented JPEG should decode")
                .dimensions(),
            (3, 2)
        );
    }

    #[test]
    fn missing_effect_slots_are_transparent() {
        let atlas = effect_atlas(&[None, None, None, None]).expect("empty atlas should build");
        assert_eq!(atlas.dimensions(), (512, 512));
        assert!(atlas.pixels().all(|pixel| pixel.0 == [0, 0, 0, 0]));
    }

    #[test]
    fn malformed_ranges_and_missing_chunks_are_rejected() {
        let malformed = 1..9;
        assert!(validate_chunks(b"short", std::slice::from_ref(&malformed)).is_err());
        let directory = tempdir().expect("temporary directory should be created");
        let source = directory.path().join("empty.afb");
        fs::write(&source, b"no embedded textures").expect("fixture should be written");
        assert!(matches!(
            extract_dds(&source, &directory.path().join("output")),
            Err(Error::NoDdsChunks(_))
        ));
    }
}
