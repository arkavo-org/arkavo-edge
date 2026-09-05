#![allow(clippy::disallowed_methods)]

use arkavo_llm::providers::openai::{OpenAIConfig, OpenAIProvider};
use arkavo_llm::{Message, Provider, Role};
use std::time::Instant;

#[path = "common/mod.rs"]
mod common;
use common::ensure_api_key;

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_gpt_4o_model() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider for GPT-5");

    let messages = vec![Message {
        response_items: Vec::new(),
        role: Role::User,
        content: "What model are you? Reply with just your model name.".to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    }];

    let start = Instant::now();
    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get response from GPT-5");
    let duration = start.elapsed();

    println!("GPT-5 response time: {duration:?}");
    println!("GPT-5 response: {response}");

    assert!(!response.is_empty());
    assert!(response.to_lowercase().contains("gpt") || response.contains("4o"));
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_gpt_4_turbo_model() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider for GPT-5");

    let messages = vec![Message {
        response_items: Vec::new(),
        role: Role::User,
        content: "Generate a haiku about Rust programming. Reply with just the haiku.".to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    }];

    let start = Instant::now();
    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get response from GPT-5");
    let duration = start.elapsed();

    println!("GPT-5 response time: {duration:?}");
    println!("GPT-5 haiku:\n{response}");

    assert!(!response.is_empty());
    assert!(
        response.trim().lines().count() >= 3,
        "Haiku should have at least 3 lines"
    );
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_gpt_35_turbo_model() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider for GPT-5");

    let messages = vec![Message {
        response_items: Vec::new(),
        role: Role::User,
        content: "What is 2 + 2? Reply with just the number.".to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    }];

    let start = Instant::now();
    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get response from GPT-5");
    let duration = start.elapsed();

    println!("GPT-5 response time: {duration:?}");
    println!("GPT-5 response: {response}");

    assert!(response.contains('4'));
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_model_performance_comparison() {
    let api_key = ensure_api_key();

    let models = vec![
        ("gpt-5", "GPT-5"),
        ("gpt-5", "GPT-5 (test 2)"),
        ("gpt-5", "GPT-5 (test 3)"),
    ];

    let test_prompt = "Write a one-line description of the Rust programming language.";

    println!("\n=== Model Performance Comparison ===\n");

    for (model_id, model_name) in models {
        let config = OpenAIConfig {
            api_key: api_key.clone(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: model_id.to_string(),
            organization_id: None,
            api_version: None,
            is_azure: false,
        };

        match OpenAIProvider::new(config) {
            Ok(provider) => {
                let messages = vec![Message {
                    response_items: Vec::new(),
                    role: Role::User,
                    content: test_prompt.to_string(),
                    images: None,
                    tool_call_id: None,
                    tool_name: None,
                    tool_calls: Vec::new(),
                }];

                let start = Instant::now();
                match provider.complete(messages).await {
                    Ok(response) => {
                        let duration = start.elapsed();
                        println!("Model: {model_name}");
                        println!("Response time: {duration:?}");
                        println!("Response: {}", response.trim());
                        println!("---");
                    }
                    Err(e) => {
                        println!("Model: {model_name} - Error: {e}");
                        println!("---");
                    }
                }
            }
            Err(e) => {
                println!("Failed to create provider for {model_name}: {e}");
            }
        }
    }
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_context_window_handling() {
    let api_key = ensure_api_key();

    let config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(), // 128k context window
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Failed to create OpenAI provider");

    // Create a long context
    let long_text = "The quick brown fox jumps over the lazy dog. ".repeat(100);

    let messages = vec![
        Message {
            response_items: Vec::new(),
            role: Role::System,
            content: format!("Remember this text: {long_text}"),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        },
        Message {
            response_items: Vec::new(),
            role: Role::User,
            content:
                "How many times does the word 'fox' appear in the text I asked you to remember?"
                    .to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        },
    ];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get response with long context");

    assert!(response.contains("100") || response.to_lowercase().contains("hundred"));
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_json_mode_response() {
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
            response_items: Vec::new(),
            role: Role::System,
            content: "You must respond with valid JSON only.".to_string(),
            images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
        },
        Message {
            response_items: Vec::new(),
            role: Role::User,
            content: r#"Create a JSON object with fields: "name" (string), "age" (number), and "active" (boolean)."#.to_string(),
            images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
        },
    ];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get JSON response");

    // Try to parse as JSON
    let json_result: Result<serde_json::Value, _> = serde_json::from_str(&response);
    assert!(
        json_result.is_ok(),
        "Response should be valid JSON: {response}"
    );

    let json = json_result.unwrap();
    assert!(json.get("name").is_some());
    assert!(json.get("age").is_some());
    assert!(json.get("active").is_some());
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_model_fallback() {
    let api_key = ensure_api_key();

    // Try with a non-existent model first
    let config = OpenAIConfig {
        api_key: api_key.clone(),
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5-ultra".to_string(), // Non-existent model
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider = OpenAIProvider::new(config).expect("Provider creation should succeed");

    let messages = vec![Message {
        response_items: Vec::new(),
        role: Role::User,
        content: "Hello".to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    }];

    let result = provider.complete(messages.clone()).await;
    assert!(result.is_err());
    let error_msg = result.unwrap_err().to_string();
    assert!(error_msg.contains("model") || error_msg.contains("404"));

    // Fallback to a valid model
    let fallback_config = OpenAIConfig {
        api_key,
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let fallback_provider =
        OpenAIProvider::new(fallback_config).expect("Failed to create fallback provider");

    let response = fallback_provider
        .complete(messages)
        .await
        .expect("Fallback model should work");

    assert!(!response.is_empty());
}
