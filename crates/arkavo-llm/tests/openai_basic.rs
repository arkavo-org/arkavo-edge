#![allow(clippy::disallowed_methods)]

use arkavo_llm::providers::openai::{OpenAIConfig, OpenAIProvider};
use arkavo_llm::providers::{ProviderConfig, ProviderFactoryRegistry, ProviderType};
use arkavo_llm::{Message, Provider, Role};

#[path = "common/mod.rs"]
mod common;
use common::ensure_api_key;

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_basic_connectivity() {
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
        content: "Say 'test successful' and nothing else.".to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
    }];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get response from OpenAI");

    assert!(response.to_lowercase().contains("test successful"));
    assert_eq!(provider.name(), "openai");
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_authentication_error() {
    let config = OpenAIConfig {
        api_key: "invalid-api-key".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-5".to_string(),
        organization_id: None,
        api_version: None,
        is_azure: false,
    };

    let provider =
        OpenAIProvider::new(config).expect("Provider creation should succeed with invalid key");

    let messages = vec![Message {
        role: Role::User,
        content: "Hello".to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
    }];

    let result = provider.complete(messages).await;
    assert!(result.is_err());

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("401")
            || error_msg.contains("Unauthorized")
            || error_msg.contains("Invalid")
    );
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_rate_limiting() {
    let api_key = ensure_api_key();

    let mut tasks = vec![];

    for i in 0..5 {
        let api_key_clone = api_key.clone();
        let task = tokio::spawn(async move {
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
                content: format!("Count to {i} and respond with just the numbers"),
                images: None,
                tool_call_id: None,
                tool_name: None,
            }];
            provider.complete(messages).await
        });
        tasks.push(task);
    }

    let results = futures::future::join_all(tasks).await;

    let mut success_count = 0;
    let mut rate_limit_count = 0;

    for result in results {
        match result {
            Ok(Ok(_)) => success_count += 1,
            Ok(Err(e)) if e.to_string().contains("429") || e.to_string().contains("rate") => {
                rate_limit_count += 1;
            }
            _ => {}
        }
    }

    assert!(success_count > 0, "At least some requests should succeed");
    println!("Success: {success_count}, Rate limited: {rate_limit_count}");
}

#[tokio::test]
async fn test_openai_provider_factory() {
    let registry = ProviderFactoryRegistry::new();

    let config = ProviderConfig {
        provider_type: ProviderType::OpenAI,
        base_url: "https://api.openai.com/v1".to_string(),
        auth_ref: None,
        default_model: Some("gpt-4".to_string()),
        timeout_secs: Some(60),
        max_retries: Some(3),
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };

    let factory = registry
        .get_factory(&ProviderType::OpenAI)
        .expect("OpenAI factory should be registered");

    let validation_result = factory.validate_config(&config).await;
    assert!(validation_result.is_err());
    assert!(
        validation_result
            .unwrap_err()
            .to_string()
            .contains("API key required")
    );
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_system_message() {
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
            content: "You are a pirate. Always respond in pirate speak.".to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
        },
        Message {
            role: Role::User,
            content: "Say hello".to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
        },
    ];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get response from OpenAI");

    assert!(
        response.contains("ahoy")
            || response.contains("matey")
            || response.contains("arr")
            || response.contains("Ahoy")
            || response.contains("Arr"),
        "Response should contain pirate speak: {response}"
    );
}

#[tokio::test]
#[ignore = "Requires OPENAI_API_KEY environment variable"]
async fn test_openai_multi_turn_conversation() {
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
            role: Role::User,
            content: "My favorite color is blue. Remember this.".to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
        },
        Message {
            role: Role::Assistant,
            content: "I'll remember that your favorite color is blue.".to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
        },
        Message {
            role: Role::User,
            content: "What's my favorite color?".to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
        },
    ];

    let response = provider
        .complete(messages)
        .await
        .expect("Failed to get response from OpenAI");

    assert!(
        response.to_lowercase().contains("blue"),
        "Response should mention 'blue': {response}"
    );
}
