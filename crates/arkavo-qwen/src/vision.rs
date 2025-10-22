use crate::error::{QwenError, Result};
use crate::types::{ContentPart, ImageUrl};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::Path;

pub fn create_image_part(base64_data: &str) -> ContentPart {
    let url = if base64_data.starts_with("data:") {
        base64_data.to_string()
    } else {
        format!("data:image/png;base64,{base64_data}")
    };

    ContentPart::ImageUrl {
        image_url: ImageUrl { url, detail: None },
    }
}

pub fn create_image_part_with_detail(base64_data: &str, detail: &str) -> ContentPart {
    let url = if base64_data.starts_with("data:") {
        base64_data.to_string()
    } else {
        format!("data:image/png;base64,{base64_data}")
    };

    ContentPart::ImageUrl {
        image_url: ImageUrl {
            url,
            detail: Some(detail.to_string()),
        },
    }
}

pub fn create_text_part(text: impl Into<String>) -> ContentPart {
    ContentPart::Text { text: text.into() }
}

pub fn encode_image_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(STANDARD.encode(&bytes))
}

pub fn create_image_url_from_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let base64_data = STANDARD.encode(&bytes);

    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("png");

    let mime_type = match extension.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    };

    Ok(format!("data:{mime_type};base64,{base64_data}"))
}

pub fn validate_image_size(base64_data: &str, max_size_mb: usize) -> Result<()> {
    let data = if base64_data.starts_with("data:") {
        base64_data.split(',').nth(1).unwrap_or(base64_data)
    } else {
        base64_data
    };

    let decoded_size = (data.len() * 3) / 4;
    let max_size_bytes = max_size_mb * 1024 * 1024;

    if decoded_size > max_size_bytes {
        return Err(QwenError::InvalidRequest {
            message: format!(
                "Image size ({} MB) exceeds maximum allowed size ({} MB)",
                decoded_size / (1024 * 1024),
                max_size_mb
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_image_part() {
        let base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        let part = create_image_part(base64);

        match part {
            ContentPart::ImageUrl { image_url } => {
                assert!(image_url.url.starts_with("data:image/png;base64,"));
                assert_eq!(image_url.detail, None);
            }
            _ => panic!("Expected ImageUrl part"),
        }
    }

    #[test]
    fn test_create_image_part_with_existing_data_uri() {
        let data_uri = "data:image/jpeg;base64,/9j/4AAQSkZJRg==";
        let part = create_image_part(data_uri);

        match part {
            ContentPart::ImageUrl { image_url } => {
                assert_eq!(image_url.url, data_uri);
            }
            _ => panic!("Expected ImageUrl part"),
        }
    }

    #[test]
    fn test_create_image_part_with_detail() {
        let base64 = "iVBORw0KGg==";
        let part = create_image_part_with_detail(base64, "high");

        match part {
            ContentPart::ImageUrl { image_url } => {
                assert!(image_url.url.contains(base64));
                assert_eq!(image_url.detail, Some("high".to_string()));
            }
            _ => panic!("Expected ImageUrl part"),
        }
    }

    #[test]
    fn test_create_text_part() {
        let part = create_text_part("Hello, world!");

        match part {
            ContentPart::Text { text } => {
                assert_eq!(text, "Hello, world!");
            }
            _ => panic!("Expected Text part"),
        }
    }

    #[test]
    fn test_validate_image_size_valid() {
        let small_base64 = "iVBORw0KGg==";
        assert!(validate_image_size(small_base64, 10).is_ok());
    }

    #[test]
    fn test_validate_image_size_with_data_uri() {
        let data_uri = "data:image/png;base64,iVBORw0KGg==";
        assert!(validate_image_size(data_uri, 10).is_ok());
    }

    #[test]
    fn test_encode_image_file_nonexistent() {
        let result = encode_image_file(Path::new("/nonexistent/file.png"));
        assert!(result.is_err());
    }
}
