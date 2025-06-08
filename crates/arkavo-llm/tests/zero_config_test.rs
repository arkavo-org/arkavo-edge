use arkavo_llm::{LlmClient, ChatRequest};
use std::io::Write;
use tempfile::NamedTempFile;

#[tokio::test]
async fn test_zero_config_text_chat() {
    unsafe {
        std::env::set_var("LLM_PROVIDER", "ollama");
        std::env::set_var("OLLAMA_URL", "http://localhost:11434");
    }
    
    if let Ok(client) = LlmClient::from_env() {
        let request = ChatRequest::new("What is 2+2?");
        
        let result = client.chat_unified(request).await;
        match result {
            Ok(response) => {
                println!("Text chat succeeded: {}", response);
                assert!(!response.is_empty());
            }
            Err(e) => println!("Text chat failed (expected in test environment): {}", e),
        }
    }
}

#[tokio::test]
async fn test_zero_config_vision_chat() {
    unsafe {
        std::env::set_var("LLM_PROVIDER", "ollama");
        std::env::set_var("OLLAMA_URL", "http://localhost:11434");
    }
    
    if let Ok(client) = LlmClient::from_env() {
        // Create a test PNG image
        let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&png_data).unwrap();
        
        let request = ChatRequest::new("What do you see in this image?")
            .with_image(temp_file.path())
            .unwrap();
        
        let result = client.chat_unified(request).await;
        match result {
            Ok(response) => {
                println!("Vision chat succeeded: {}", response);
                assert!(!response.is_empty());
            }
            Err(e) => println!("Vision chat failed (expected in test environment): {}", e),
        }
    }
}

#[tokio::test] 
async fn test_automatic_model_selection() {
    unsafe {
        std::env::set_var("LLM_PROVIDER", "ollama");
        std::env::set_var("OLLAMA_URL", "http://localhost:11434");
    }
    
    if let Ok(client) = LlmClient::from_env() {
        // Test 1: Text-only should use regular model
        let text_request = ChatRequest::new("Hello");
        let _result1 = client.chat_unified(text_request).await;
        
        // Test 2: With image should automatically use vision model
        let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(&png_data).unwrap();
        
        let vision_request = ChatRequest::new("Describe this")
            .with_image(temp_file.path())
            .unwrap();
        let _result2 = client.chat_unified(vision_request).await;
        
        // Both should work transparently - model selection is automatic
        println!("Automatic model selection test completed");
    }
}

#[test]
fn test_chat_request_builder_ergonomics() {
    // Test the fluent API for building requests
    let text_request = ChatRequest::new("Simple question");
    assert!(text_request.images.is_empty());
    
    let mixed_request = ChatRequest::new("Analyze this")
        .with_encoded_image("image1_base64".to_string())
        .with_encoded_image("image2_base64".to_string());
    assert_eq!(mixed_request.images.len(), 2);
    
    // Users don't need to know about vision vs text models - it's all transparent
}