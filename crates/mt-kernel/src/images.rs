//! Decoding images that come from files (textures, photos, thumbnails, icons) with
//! limits, so a damaged or hostile image gives an error instead of a huge
//! allocation: a PNG header can claim 65535 × 65535 pixels in a few bytes, and the
//! decoder allocates the whole picture before reading any pixel data.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageDecoder, ImageError, ImageReader, ImageResult};

/// Largest width or height.
pub const MAX_SIDE: u32 = 16384;
/// Most pixels (8192 × 8192): the RGBA8 picture callers make stays within 256 MiB.
pub const MAX_PIXELS: u64 = 8192 * 8192;
/// Most bytes the decoder may allocate at once.
pub const MAX_ALLOC: u64 = 256 << 20;

/// The decoder limits used here.
pub fn limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(MAX_SIDE);
    l.max_image_height = Some(MAX_SIDE);
    l.max_alloc = Some(MAX_ALLOC);
    l
}

fn too_large() -> ImageError {
    ImageError::Limits(image::error::LimitError::from_kind(
        image::error::LimitErrorKind::DimensionError,
    ))
}

fn decode_reader(mut reader: ImageReader<Cursor<&[u8]>>) -> ImageResult<DynamicImage> {
    reader.limits(limits());
    let decoder = reader.into_decoder()?;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(too_large());
    }
    DynamicImage::from_decoder(decoder)
}

/// Decodes image bytes of any format the build supports (as
/// `image::load_from_memory`), within the limits.
pub fn decode(data: &[u8]) -> ImageResult<DynamicImage> {
    decode_reader(ImageReader::new(Cursor::new(data)).with_guessed_format()?)
}

/// Decodes image bytes of a known format within the limits.
pub fn decode_with_format(data: &[u8], format: image::ImageFormat) -> ImageResult<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(data));
    reader.set_format(format);
    decode_reader(reader)
}

/// Reads and decodes an image file within the limits (as `image::open`).
pub fn open(path: impl AsRef<Path>) -> ImageResult<DynamicImage> {
    let path = path.as_ref();
    let data = std::fs::read(path).map_err(ImageError::IoError)?;
    let format = image::ImageFormat::from_path(path).ok();
    match format {
        // The content decides when it disagrees with the extension, as `image::open`
        // does not; a guess from the bytes first, the extension as a fallback.
        _ if image::guess_format(&data).is_ok() => decode(&data),
        Some(f) => decode_with_format(&data, f),
        None => decode(&data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::new(w, h);
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn decodes_small_images() {
        let img = decode(&png(5, 3)).unwrap();
        assert_eq!((img.width(), img.height()), (5, 3));
    }

    /// A 33-byte header claiming 60000 × 60000 pixels is refused before the picture
    /// is allocated.
    #[test]
    fn refuses_huge_headers() {
        let mut data = png(1, 1);
        // IHDR width and height (big endian) right after the signature and chunk
        // header; the CRC no longer matches, which the decoder checks later.
        data[16..20].copy_from_slice(&60000u32.to_be_bytes());
        data[20..24].copy_from_slice(&60000u32.to_be_bytes());
        assert!(decode(&data).is_err());
        let mut data = png(1, 1);
        data[16..20].copy_from_slice(&10000u32.to_be_bytes());
        data[20..24].copy_from_slice(&10000u32.to_be_bytes());
        assert!(decode(&data).is_err());
    }
}
