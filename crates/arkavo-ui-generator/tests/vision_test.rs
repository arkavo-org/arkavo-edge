use arkavo_ui_generator::vision::{ImageFormat, ScreenshotCapture};
use base64::Engine;
use std::fs;
use tempfile::NamedTempFile;

#[test]
fn test_png_from_file() {
    let png_data = create_minimal_png();

    let temp_file = NamedTempFile::new().unwrap();
    fs::write(temp_file.path(), &png_data).unwrap();

    let screenshot = ScreenshotCapture::from_file(temp_file.path()).unwrap();

    assert!(matches!(screenshot.format, ImageFormat::Png));
    assert_eq!(screenshot.width, 100);
    assert_eq!(screenshot.height, 100);
    assert!(!screenshot.image_data.is_empty());
}

#[test]
fn test_jpeg_from_file() {
    let jpeg_data = create_minimal_jpeg();

    let temp_file = NamedTempFile::new().unwrap();
    fs::write(temp_file.path(), &jpeg_data).unwrap();

    let screenshot = ScreenshotCapture::from_file(temp_file.path()).unwrap();

    assert!(matches!(screenshot.format, ImageFormat::Jpeg));
    assert_eq!(screenshot.width, 64);
    assert_eq!(screenshot.height, 64);
}

#[test]
fn test_unsupported_format() {
    let invalid_data = vec![0x00, 0x01, 0x02, 0x03];

    let temp_file = NamedTempFile::new().unwrap();
    fs::write(temp_file.path(), &invalid_data).unwrap();

    let result = ScreenshotCapture::from_file(temp_file.path());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Unsupported image format")
    );
}

#[test]
fn test_base64_encoding() {
    let png_data = create_minimal_png();

    let temp_file = NamedTempFile::new().unwrap();
    fs::write(temp_file.path(), &png_data).unwrap();

    let screenshot = ScreenshotCapture::from_file(temp_file.path()).unwrap();
    let base64 = screenshot.to_base64().unwrap();

    assert!(!base64.is_empty());
    assert!(base64.len() > 50);
}

#[test]
fn test_base64_roundtrip() {
    let png_data = create_minimal_png();

    let temp_file = NamedTempFile::new().unwrap();
    fs::write(temp_file.path(), &png_data).unwrap();

    let screenshot = ScreenshotCapture::from_file(temp_file.path()).unwrap();
    let base64 = screenshot.to_base64().unwrap();

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&base64)
        .unwrap();
    assert_eq!(decoded, png_data);
}

#[test]
fn test_create_vision_message() {
    let png_data = create_minimal_png();

    let temp_file = NamedTempFile::new().unwrap();
    fs::write(temp_file.path(), &png_data).unwrap();

    let screenshot = ScreenshotCapture::from_file(temp_file.path()).unwrap();
    let message = screenshot
        .create_vision_message("Describe this image")
        .unwrap();

    assert_eq!(message.content, "Describe this image");
    assert!(message.images.is_some());
    assert_eq!(message.images.as_ref().unwrap().len(), 1);
}

#[test]
fn test_multiple_screenshots() {
    let png_data = create_minimal_png();

    let screenshots: Vec<_> = (0..3)
        .map(|_| {
            let temp_file = NamedTempFile::new().unwrap();
            fs::write(temp_file.path(), &png_data).unwrap();
            ScreenshotCapture::from_file(temp_file.path()).unwrap()
        })
        .collect();

    assert_eq!(screenshots.len(), 3);
    for screenshot in screenshots {
        assert_eq!(screenshot.width, 100);
        assert_eq!(screenshot.height, 100);
    }
}

fn create_minimal_png() -> Vec<u8> {
    let mut png = Vec::new();

    png.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

    png.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]);
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0x64, 0x08, 0x02, 0x00, 0x00, 0x00,
    ]);

    let mut crc = crc32_simple(
        b"IHDR",
        &[
            0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0x64, 0x08, 0x02, 0x00, 0x00, 0x00,
        ],
    );
    png.extend_from_slice(&crc.to_be_bytes());

    let idat_data = vec![0x08, 0x1D, 0x01, 0x02, 0x00, 0xFD, 0xFF, 0x00, 0x00];
    png.extend_from_slice(&(idat_data.len() as u32).to_be_bytes());
    png.extend_from_slice(b"IDAT");
    png.extend_from_slice(&idat_data);
    crc = crc32_simple(b"IDAT", &idat_data);
    png.extend_from_slice(&crc.to_be_bytes());

    png.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    png.extend_from_slice(b"IEND");
    crc = crc32_simple(b"IEND", &[]);
    png.extend_from_slice(&crc.to_be_bytes());

    png
}

fn create_minimal_jpeg() -> Vec<u8> {
    let mut jpeg = Vec::new();

    jpeg.extend_from_slice(&[0xFF, 0xD8]);

    jpeg.extend_from_slice(&[0xFF, 0xE0]);
    jpeg.extend_from_slice(&[0x00, 0x10]);
    jpeg.extend_from_slice(b"JFIF");
    jpeg.extend_from_slice(&[0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);

    jpeg.extend_from_slice(&[0xFF, 0xC0]);
    jpeg.extend_from_slice(&[0x00, 0x11]);
    jpeg.extend_from_slice(&[0x08]);
    jpeg.extend_from_slice(&[0x00, 0x40]);
    jpeg.extend_from_slice(&[0x00, 0x40]);
    jpeg.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);

    jpeg.extend_from_slice(&[0xFF, 0xDA]);
    jpeg.extend_from_slice(&[0x00, 0x0C]);
    jpeg.extend_from_slice(&[0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00]);

    jpeg.extend_from_slice(&[0x00; 100]);

    jpeg.extend_from_slice(&[0xFF, 0xD9]);

    jpeg
}

fn crc32_simple(chunk_type: &[u8], data: &[u8]) -> u32 {
    const CRC_TABLE: [u32; 256] = generate_crc_table();

    let mut crc = 0xFFFFFFFFu32;

    for &byte in chunk_type.iter().chain(data.iter()) {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC_TABLE[index] ^ (crc >> 8);
    }

    crc ^ 0xFFFFFFFF
}

const fn generate_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;

    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;

        while k < 8 {
            if c & 1 != 0 {
                c = 0xEDB88320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }

        table[n] = c;
        n += 1;
    }

    table
}
