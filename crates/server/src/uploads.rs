//! Safe handling of uploaded images.
//!
//! Uploads are untrusted. The pipeline is: check the real file type from
//! its magic bytes (never the file name or Content-Type), decode with hard
//! dimension and memory limits on a blocking thread with a timeout, apply
//! the EXIF orientation, then re-encode. Re-encoding drops every metadata
//! chunk, so EXIF data such as GPS coordinates never reaches storage.

use std::io::Cursor;
use std::time::Duration;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};

pub const MAX_DIMENSION: u32 = 6000;
pub const MAX_PIXELS: u64 = 24_000_000;
const DECODE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadError {
    UnsupportedType,
    TooLarge,
    Invalid,
}

impl UploadError {
    pub fn code(&self) -> &'static str {
        match self {
            UploadError::UnsupportedType => "upload_type",
            UploadError::TooLarge => "upload_too_large",
            UploadError::Invalid => "upload_invalid",
        }
    }
}

/// Identify PNG, JPEG or WebP by signature.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::WebP)
    } else {
        None
    }
}

/// Decode with limits and apply EXIF orientation. Blocking; call through
/// [`decode`].
pub fn decode_blocking(bytes: &[u8]) -> Result<DynamicImage, UploadError> {
    let format = sniff(bytes).ok_or(UploadError::UnsupportedType)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    // Caps decoder allocations, which defuses decompression bombs.
    limits.max_alloc = Some(MAX_PIXELS * 4 + 64 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(map_image_error)?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 {
        return Err(UploadError::Invalid);
    }
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(UploadError::TooLarge);
    }
    // Phones store photos sideways plus an orientation tag; bake it in,
    // because the tag is lost when we re-encode.
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(map_image_error)?;
    img.apply_orientation(orientation);
    Ok(img)
}

fn map_image_error(e: image::ImageError) -> UploadError {
    match e {
        image::ImageError::Limits(_) => UploadError::TooLarge,
        _ => UploadError::Invalid,
    }
}

/// Decode off the async runtime with a time limit.
pub async fn decode(bytes: bytes::Bytes) -> Result<DynamicImage, UploadError> {
    let task = tokio::task::spawn_blocking(move || decode_blocking(&bytes));
    match tokio::time::timeout(DECODE_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        // Join error (decoder panicked) or timeout: treat as a bad file.
        Ok(Err(_)) | Err(_) => Err(UploadError::Invalid),
    }
}

pub fn encode_png(img: &DynamicImage) -> Result<Vec<u8>, UploadError> {
    let mut buf = Vec::new();
    // Normalise to 8-bit RGBA so the renderer has one format to handle.
    DynamicImage::ImageRgba8(img.to_rgba8()).write_to(&mut Cursor::new(&mut buf), ImageFormat::Png).map_err(|_| UploadError::Invalid)?;
    Ok(buf)
}

pub fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, UploadError> {
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality).encode_image(&img.to_rgb8()).map_err(|_| UploadError::Invalid)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        encode_png(&DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255])))).unwrap()
    }

    #[test]
    fn sniffs_by_magic_bytes_not_by_name() {
        assert_eq!(sniff(&png(2, 2)), Some(ImageFormat::Png));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageFormat::Jpeg));
        assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some(ImageFormat::WebP));
        assert_eq!(sniff(b"GIF89a"), None);
        assert_eq!(sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(sniff(b"%PDF-1.4"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn decodes_valid_and_rejects_corrupt() {
        let img = decode_blocking(&png(8, 6)).unwrap();
        assert_eq!((img.width(), img.height()), (8, 6));
        let mut broken = png(8, 6);
        broken.truncate(30);
        assert_eq!(decode_blocking(&broken).err(), Some(UploadError::Invalid));
        assert_eq!(decode_blocking(b"hello").err(), Some(UploadError::UnsupportedType));
    }

    #[test]
    fn rejects_decompression_bombs_by_header() {
        // A PNG header that claims 60000 x 60000 pixels: refused from the
        // header alone, without allocating a pixel buffer.
        let mut bomb = png(1, 1);
        bomb[16..20].copy_from_slice(&60_000u32.to_be_bytes());
        bomb[20..24].copy_from_slice(&60_000u32.to_be_bytes());
        assert!(decode_blocking(&bomb).is_err());
    }

    /// A JPEG with an EXIF block that says "rotate 90°" and has a GPS tag.
    fn jpeg_with_exif() -> Vec<u8> {
        let plain = encode_jpeg(&DynamicImage::ImageRgb8(image::RgbImage::from_pixel(40, 20, image::Rgb([200, 10, 10]))), 90).unwrap();
        // TIFF header + IFD0 with Orientation = 6 (rotate 90° clockwise).
        let mut tiff: Vec<u8> = Vec::new();
        tiff.extend_from_slice(b"II*\x00\x08\x00\x00\x00"); // little endian, IFD at 8
        tiff.extend_from_slice(&1u16.to_le_bytes()); // one entry
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_le_bytes()); // SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&6u16.to_le_bytes());
        tiff.extend_from_slice(&0u16.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
        tiff.extend_from_slice(b"GPS-SECRET-41.2995N-69.2401E");
        let mut app1 = b"Exif\x00\x00".to_vec();
        app1.extend_from_slice(&tiff);
        let mut out = plain[..2].to_vec(); // SOI
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&app1);
        out.extend_from_slice(&plain[2..]);
        out
    }

    #[test]
    fn exif_orientation_is_applied_and_metadata_is_stripped() {
        let src = jpeg_with_exif();
        assert!(src.windows(10).any(|w| w == b"GPS-SECRET"));
        let img = decode_blocking(&src).unwrap();
        // 40x20 rotated by 90° becomes 20x40.
        assert_eq!((img.width(), img.height()), (20, 40));
        for out in [encode_png(&img).unwrap(), encode_jpeg(&img, 85).unwrap()] {
            assert!(!out.windows(10).any(|w| w == b"GPS-SECRET"), "metadata leaked into the re-encoded file");
            assert!(!out.windows(4).any(|w| w == b"Exif"));
        }
    }

    #[tokio::test]
    async fn async_decode_wrapper() {
        assert!(decode(bytes::Bytes::from(png(4, 4))).await.is_ok());
        assert_eq!(decode(bytes::Bytes::from_static(b"nope")).await.err(), Some(UploadError::UnsupportedType));
    }
}
