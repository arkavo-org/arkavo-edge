use crate::{LlmClient, Result, encode_image_file};
use std::path::Path;

pub struct ChatRequest {
    pub content: String,
    pub images: Vec<String>,
}

impl ChatRequest {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn with_image(mut self, image_path: impl AsRef<Path>) -> Result<Self> {
        let encoded = encode_image_file(image_path)?;
        self.images.push(encoded);
        Ok(self)
    }

    pub fn with_images(mut self, image_paths: Vec<impl AsRef<Path>>) -> Result<Self> {
        for path in image_paths {
            let encoded = encode_image_file(path)?;
            self.images.push(encoded);
        }
        Ok(self)
    }

    pub fn with_encoded_image(mut self, encoded_image: String) -> Self {
        self.images.push(encoded_image);
        self
    }

    pub fn with_encoded_images(mut self, encoded_images: Vec<String>) -> Self {
        self.images.extend(encoded_images);
        self
    }
}

impl LlmClient {
    pub async fn chat_unified(&self, request: ChatRequest) -> Result<String> {
        let message = if request.images.is_empty() {
            crate::Message::user(request.content)
        } else {
            crate::Message::user_with_images(request.content, request.images)
        };

        self.complete(vec![message]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_chat_request_text_only() {
        let request = ChatRequest::new("Hello, world!");
        assert_eq!(request.content, "Hello, world!");
        assert!(request.images.is_empty());
    }

    #[test]
    fn test_chat_request_with_encoded_image() {
        let request = ChatRequest::new("Describe this")
            .with_encoded_image("base64encodedimage".to_string());
        
        assert_eq!(request.content, "Describe this");
        assert_eq!(request.images.len(), 1);
        assert_eq!(request.images[0], "base64encodedimage");
    }

    #[test]
    fn test_chat_request_with_image_file() {
        let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&png_data).unwrap();

        let request = ChatRequest::new("What's in this image?")
            .with_image(temp_file.path())
            .unwrap();

        assert_eq!(request.content, "What's in this image?");
        assert_eq!(request.images.len(), 1);
        assert!(!request.images[0].is_empty());
    }

    #[test]
    fn test_chat_request_builder_pattern() {
        let request = ChatRequest::new("Analyze these images")
            .with_encoded_image("image1".to_string())
            .with_encoded_image("image2".to_string());

        assert_eq!(request.images.len(), 2);
        assert_eq!(request.images[0], "image1");
        assert_eq!(request.images[1], "image2");
    }
}