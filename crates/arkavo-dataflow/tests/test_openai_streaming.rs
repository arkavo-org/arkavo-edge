use arkavo_dataflow::nodes::openai_provider::{OpenAIConfig, OpenAIProvider};
use arkavo_llm::{Message, Provider, Role};
use futures::StreamExt;
use std::time::Instant;

#[path = "mod.rs"]
mod common;
use common::ensure_api_key;

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_streaming_basic() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    let messages = vec![Message {
        role: Role::User,
        content: "List the numbers from 1 to 5, separated by spaces.".to_string(),
        images: None,
    }];

    let mut stream = provider
        .stream(messages)
        .await
        .expect("Failed to create stream");

    let mut chunks = Vec::new();
    let mut chunk_count = 0;

    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                chunk_count += 1;
                chunks.push(response.content.clone());

                if response.done {
                    break;
                }
            }
            Err(e) => {
                panic!("Stream error: {}", e);
            }
        }
    }

    let full_response = chunks.join("");
    println!("Received {} chunks", chunk_count);
    println!("Full response: {}", full_response);

    assert!(chunk_count > 1, "Should receive multiple chunks");
    assert!(!full_response.is_empty(), "Should receive a response");
    assert!(
        full_response.chars().any(|c| c.is_numeric()),
        "Response should contain numbers: '{}'",
        full_response
    );
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_streaming_performance() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    let messages = vec![Message {
        role: Role::User,
        content: "Write a 3-sentence story about a robot.".to_string(),
        images: None,
    }];

    let start = Instant::now();
    let mut stream = provider
        .stream(messages)
        .await
        .expect("Failed to create stream");

    let mut first_chunk_time = None;
    let mut total_chunks = 0;
    let mut content = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                if first_chunk_time.is_none() && !response.content.is_empty() {
                    first_chunk_time = Some(start.elapsed());
                }

                content.push_str(&response.content);
                total_chunks += 1;

                if response.done {
                    break;
                }
            }
            Err(e) => {
                panic!("Stream error: {}", e);
            }
        }
    }

    let total_time = start.elapsed();

    println!("First chunk received in: {:?}", first_chunk_time.unwrap());
    println!("Total streaming time: {:?}", total_time);
    println!("Total chunks: {}", total_chunks);
    println!("Content length: {} chars", content.len());

    assert!(
        first_chunk_time.unwrap().as_millis() < 5000,
        "First chunk should arrive within 5 seconds"
    );
    assert!(total_chunks > 1, "Should receive multiple chunks");
    assert!(!content.is_empty(), "Should receive content");
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_streaming_interruption() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    let messages = vec![Message {
        role: Role::User,
        content: "Count from 1 to 100, one number per line.".to_string(),
        images: None,
    }];

    let mut stream = provider
        .stream(messages)
        .await
        .expect("Failed to create stream");

    let mut chunks_received = 0;
    const MAX_CHUNKS: usize = 5;

    while let Some(result) = stream.next().await {
        match result {
            Ok(_response) => {
                chunks_received += 1;

                if chunks_received >= MAX_CHUNKS {
                    println!("Interrupting stream after {} chunks", chunks_received);
                    break; // Simulate interruption
                }
            }
            Err(e) => {
                panic!("Stream error: {}", e);
            }
        }
    }

    assert_eq!(chunks_received, MAX_CHUNKS, "Should stop after max chunks");
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_streaming_with_system_message() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    let messages = vec![
        Message {
            role: Role::System,
            content: "You are a poet. Respond only in haiku format.".to_string(),
            images: None,
        },
        Message {
            role: Role::User,
            content: "Describe streaming data".to_string(),
            images: None,
        },
    ];

    let mut stream = provider
        .stream(messages)
        .await
        .expect("Failed to create stream");

    let mut full_response = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(response) => {
                full_response.push_str(&response.content);

                if response.done {
                    break;
                }
            }
            Err(e) => {
                panic!("Stream error: {}", e);
            }
        }
    }

    println!("Haiku response:\n{}", full_response);

    let lines: Vec<&str> = full_response.trim().lines().collect();
    assert!(lines.len() >= 3, "Haiku should have at least 3 lines");
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_streaming_concurrent() {
    let api_key = ensure_api_key();

    // Create multiple concurrent streaming requests
    let prompts = vec![
        "Please respond with exactly: Stream 1 OK",
        "Please respond with exactly: Stream 2 OK",
        "Please respond with exactly: Stream 3 OK",
    ];

    let mut handles = vec![];

    for prompt in prompts {
        let api_key_clone = api_key.clone();
        let prompt_owned = prompt.to_string();

        let handle = tokio::spawn(async move {
            // Create a new provider for each concurrent request
            let config = OpenAIConfig {
                api_key: api_key_clone,
                base_url: "https://api.openai.com/v1".to_string(),
                model: "gpt-5".to_string(),
                organization_id: None,
                api_version: None,
                is_azure: false,
            };

            let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

            let messages = vec![Message {
                role: Role::User,
                content: prompt_owned.clone(),
                images: None,
            }];

            let mut stream = provider
                .stream(messages)
                .await
                .expect("Failed to create stream");

            let mut content = String::new();

            while let Some(result) = stream.next().await {
                match result {
                    Ok(response) => {
                        content.push_str(&response.content);
                        if response.done {
                            break;
                        }
                    }
                    Err(e) => {
                        return Err(format!("Stream error: {}", e));
                    }
                }
            }

            Ok(content)
        });

        handles.push(handle);
    }

    // Wait for all streams to complete
    let results = futures::future::join_all(handles).await;

    for (i, result) in results.iter().enumerate() {
        match result {
            Ok(Ok(content)) => {
                println!("Stream {} response: {}", i + 1, content);
                assert!(
                    content.contains("Stream") || 
                    content.contains("OK") || 
                    content.contains(&(i + 1).to_string()),
                    "Response should reference stream number or OK, got: '{}'",
                    content
                );
            }
            _ => panic!("Stream {} failed", i + 1),
        }
    }
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_streaming_error_handling() {
    // Test with invalid model to trigger an error
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "invalid-model-name".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Provider creation should succeed");

    let messages = vec![Message {
        role: Role::User,
        content: "Hello".to_string(),
        images: None,
    }];

    let result = provider.stream(messages).await;

    match result {
        Err(e) => {
            let error = e.to_string();
            assert!(
                error.contains("404") || error.contains("model") || error.contains("not found")
            );
        }
        Ok(_) => panic!("Should fail with invalid model"),
    }
}
