use arkavo_llm::{LlmClient, Message, encode_image_file, encode_image_bytes, ImageFormat};
use base64::prelude::*;
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_message_with_images_construction() {
    let images = vec!["base64encoded1".to_string(), "base64encoded2".to_string()];
    let message = Message::user_with_images("Describe these images", images.clone());
    
    assert_eq!(message.content, "Describe these images");
    assert_eq!(message.images, Some(images));
}

#[test]
fn test_image_encoding_png() {
    let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let encoded = encode_image_bytes(&png_header).unwrap();
    assert!(!encoded.is_empty());
    
    let decoded = BASE64_STANDARD.decode(&encoded).unwrap();
    assert_eq!(png_header.to_vec(), decoded);
}

#[test]
fn test_image_encoding_jpeg() {
    let jpeg_header = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
    let encoded = encode_image_bytes(&jpeg_header).unwrap();
    assert!(!encoded.is_empty());
}

#[test]
fn test_image_format_validation() {
    let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    assert!(matches!(ImageFormat::validate_bytes(&png_data), Ok(ImageFormat::Png)));
    
    let jpeg_data = [0xFF, 0xD8, 0xFF, 0xE0];
    assert!(matches!(ImageFormat::validate_bytes(&jpeg_data), Ok(ImageFormat::Jpeg)));
    
    let invalid_data = [0x00, 0x00, 0x00, 0x00];
    assert!(ImageFormat::validate_bytes(&invalid_data).is_err());
}

#[test]
fn test_encode_image_file_with_temp_file() {
    let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    
    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(&png_data).unwrap();
    
    let encoded = encode_image_file(temp_file.path()).unwrap();
    assert!(!encoded.is_empty());
    
    let decoded = BASE64_STANDARD.decode(&encoded).unwrap();
    assert_eq!(png_data.to_vec(), decoded);
}

#[tokio::test]
async fn test_llm_client_vision_methods_interface() {
    unsafe {
        std::env::set_var("LLM_PROVIDER", "ollama");
        std::env::set_var("OLLAMA_URL", "http://localhost:11434");
    }
    
    if let Ok(client) = LlmClient::from_env() {
        let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&png_data).unwrap();
        
        let encoded_images = vec![encode_image_file(temp_file.path()).unwrap()];
        
        let result = client.complete_with_encoded_images("What is this?", encoded_images).await;
        match result {
            Ok(_) => println!("Vision completion succeeded"),
            Err(e) => println!("Vision completion failed (expected in test): {}", e),
        }
    }
}

#[test]
fn test_message_serialization_compatibility() {
    let text_only = Message::user("Hello");
    let json = serde_json::to_string(&text_only).unwrap();
    assert!(!json.contains("images"));
    
    let with_images = Message::user_with_images("Hello", vec!["img".to_string()]);
    let json = serde_json::to_string(&with_images).unwrap();
    assert!(json.contains("images"));
    assert!(json.contains("img"));
}

#[test]
fn test_backward_compatibility() {
    let legacy_json = r#"{"role":"user","content":"Hello"}"#;
    let message: Message = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(message.content, "Hello");
    assert_eq!(message.images, None);
    
    let modern_json = r#"{"role":"user","content":"Hello","images":["img1"]}"#;
    let message: Message = serde_json::from_str(modern_json).unwrap();
    assert_eq!(message.content, "Hello");
    assert_eq!(message.images, Some(vec!["img1".to_string()]));
}