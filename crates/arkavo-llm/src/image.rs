use crate::{Error, Result};
use base64::prelude::*;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
}

impl ImageFormat {
    pub fn from_path(path: &Path) -> Result<Self> {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .ok_or_else(|| Error::InvalidImageFormat("Missing file extension".to_string()))?;

        match extension.to_lowercase().as_str() {
            "png" => Ok(ImageFormat::Png),
            "jpg" | "jpeg" => Ok(ImageFormat::Jpeg),
            "webp" => Ok(ImageFormat::WebP),
            _ => Err(Error::InvalidImageFormat(format!(
                "Unsupported image format: {}",
                extension
            ))),
        }
    }

    pub fn validate_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 4 {
            return Err(Error::InvalidImageFormat("File too small".to_string()));
        }

        match &bytes[..4] {
            [0x89, 0x50, 0x4E, 0x47] => Ok(ImageFormat::Png),
            [0xFF, 0xD8, 0xFF, _] => Ok(ImageFormat::Jpeg),
            _ if bytes.len() >= 12 && &bytes[8..12] == b"WEBP" => Ok(ImageFormat::WebP),
            _ => Err(Error::InvalidImageFormat(
                "Unknown or unsupported image format".to_string(),
            )),
        }
    }
}

pub fn encode_image_file(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    
    if !path.exists() {
        return Err(Error::InvalidImagePath(format!(
            "Image file not found: {}",
            path.display()
        )));
    }

    let bytes = fs::read(path).map_err(|e| {
        Error::InvalidImagePath(format!("Failed to read image file: {}", e))
    })?;

    ImageFormat::validate_bytes(&bytes)?;
    
    Ok(BASE64_STANDARD.encode(&bytes))
}

pub fn encode_image_bytes(bytes: &[u8]) -> Result<String> {
    ImageFormat::validate_bytes(bytes)?;
    Ok(BASE64_STANDARD.encode(bytes))
}

pub fn decode_image(encoded: &str) -> Result<Vec<u8>> {
    let bytes = BASE64_STANDARD.decode(encoded)
        .map_err(|e| Error::InvalidImageFormat(format!("Invalid base64: {}", e)))?;
    
    ImageFormat::validate_bytes(&bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_image_format_from_path() {
        assert!(matches!(
            ImageFormat::from_path(Path::new("test.png")),
            Ok(ImageFormat::Png)
        ));
        assert!(matches!(
            ImageFormat::from_path(Path::new("test.jpg")),
            Ok(ImageFormat::Jpeg)
        ));
        assert!(matches!(
            ImageFormat::from_path(Path::new("test.jpeg")),
            Ok(ImageFormat::Jpeg)
        ));
        assert!(matches!(
            ImageFormat::from_path(Path::new("test.webp")),
            Ok(ImageFormat::WebP)
        ));
        assert!(ImageFormat::from_path(Path::new("test.txt")).is_err());
        assert!(ImageFormat::from_path(Path::new("test")).is_err());
    }

    #[test]
    fn test_image_format_validation() {
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(matches!(
            ImageFormat::validate_bytes(&png_header),
            Ok(ImageFormat::Png)
        ));

        let jpeg_header = [0xFF, 0xD8, 0xFF, 0xE0];
        assert!(matches!(
            ImageFormat::validate_bytes(&jpeg_header),
            Ok(ImageFormat::Jpeg)
        ));

        let invalid_header = [0x00, 0x00, 0x00, 0x00];
        assert!(ImageFormat::validate_bytes(&invalid_header).is_err());

        let too_small = [0x89, 0x50];
        assert!(ImageFormat::validate_bytes(&too_small).is_err());
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let test_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        
        let encoded = encode_image_bytes(&test_data).unwrap();
        let decoded = decode_image(&encoded).unwrap();
        
        assert_eq!(test_data.to_vec(), decoded);
    }

    #[test]
    fn test_encode_image_file_not_found() {
        let result = encode_image_file("nonexistent.png");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_encode_image_file_success() {
        let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&png_data).unwrap();
        
        let encoded = encode_image_file(temp_file.path()).unwrap();
        let decoded = decode_image(&encoded).unwrap();
        
        assert_eq!(png_data.to_vec(), decoded);
    }

    #[test]
    fn test_decode_invalid_base64() {
        let result = decode_image("invalid-base64!");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid base64"));
    }
}