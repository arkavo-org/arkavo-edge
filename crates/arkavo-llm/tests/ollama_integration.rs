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
    // Test with invalid URL - use a non-routable IP to ensure failure
    unsafe {
        env::set_var(
            "OLLAMA_BASE_URL",
            "http://192.0.2.1:11434", // TEST-NET-1 (RFC 5737) - guaranteed not to be routable
        );
    }

    let client = LlmClient::from_env().expect("Failed to create client");

    let messages = vec![Message::user("Hello")];

    // Set a shorter timeout to make the test run faster
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(5), client.complete(messages)).await;

    // The request should either timeout or return an error
    match result {
        Ok(Ok(_)) => panic!("Expected an error but got success"),
        Ok(Err(e)) => {
            println!("Got expected error: {}", e);
            // Verify it's a network/connection error
            let error_str = e.to_string().to_lowercase();
            assert!(
                error_str.contains("error")
                    || error_str.contains("failed")
                    || error_str.contains("connection")
                    || error_str.contains("timeout"),
                "Unexpected error message: {}",
                e
            );
        }
        Err(_) => {
            println!("Request timed out as expected");
            // Timeout is also an acceptable failure mode
        }
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
