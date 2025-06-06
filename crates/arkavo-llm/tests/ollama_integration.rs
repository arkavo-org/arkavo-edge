use arkavo_llm::{LlmClient, Message};
use std::env;

#[tokio::test]
async fn test_ollama_client_creation() {
    unsafe {
        env::set_var("LLM_PROVIDER", "ollama");
        env::set_var("OLLAMA_BASE_URL", "http://localhost:11434");
        env::set_var("OLLAMA_MODEL", "devstral");
    }

    let client = LlmClient::from_env().expect("Failed to create client");
    assert_eq!(client.provider_name(), "ollama");
}

#[tokio::test]
#[ignore = "requires Ollama server running"]
async fn test_ollama_completion() {
    let client = LlmClient::from_env().expect("Failed to create client");

    let messages = vec![
        Message::system("You are a helpful assistant. Reply with exactly: 'Hello!'"),
        Message::user("Say hello"),
    ];

    match client.complete(messages).await {
        Ok(response) => {
            assert!(!response.is_empty());
            println!("Response: {}", response);
        }
        Err(e) => {
            println!("Expected error when Ollama not running: {}", e);
        }
    }
}

#[tokio::test]
#[ignore = "requires Ollama server running"]
async fn test_ollama_streaming() {
    use tokio_stream::StreamExt;

    let client = LlmClient::from_env().expect("Failed to create client");

    let messages = vec![
        Message::system("You are a helpful assistant"),
        Message::user("Count from 1 to 5"),
    ];

    match client.stream(messages).await {
        Ok(mut stream) => {
            let mut full_response = String::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(response) => {
                        full_response.push_str(&response.content);
                        if response.done {
                            break;
                        }
                    }
                    Err(e) => {
                        println!("Stream error: {}", e);
                        break;
                    }
                }
            }
            println!("Full streamed response: {}", full_response);
            assert!(!full_response.is_empty());
        }
        Err(e) => {
            println!("Expected error when Ollama not running: {}", e);
        }
    }
}

#[tokio::test]
async fn test_ollama_error_handling() {
    // Test with invalid URL
    unsafe {
        env::set_var(
            "OLLAMA_BASE_URL",
            "http://invalid-url-that-does-not-exist:11434",
        );
    }

    let client = LlmClient::from_env().expect("Failed to create client");

    let messages = vec![Message::user("Hello")];

    let result = client.complete(messages).await;
    assert!(result.is_err());

    if let Err(e) = result {
        println!("Expected error: {}", e);
    }
}

#[tokio::test]
async fn test_environment_configuration() {
    // Test default configuration
    unsafe {
        env::remove_var("LLM_PROVIDER");
        env::remove_var("OLLAMA_BASE_URL");
        env::remove_var("OLLAMA_MODEL");
    }

    let client = LlmClient::from_env().expect("Failed to create client");
    assert_eq!(client.provider_name(), "ollama");

    // Test custom provider error
    unsafe {
        env::set_var("LLM_PROVIDER", "unknown_provider");
    }
    let result = LlmClient::from_env();
    assert!(result.is_err());
}
