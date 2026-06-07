#![allow(clippy::disallowed_methods)]
#![allow(unexpected_cfgs)]
#![allow(unused_variables)]
#![allow(clippy::float_cmp)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::needless_collect)]

mod common;

use arkavo_llm::providers::{ProviderConfig, ProviderFactoryRegistry, ProviderType};
use arkavo_llm::{Message, Role};

#[tokio::test]
async fn test_provider_factory_registry() {
    let registry = ProviderFactoryRegistry::new();

    // Check registered providers
    let types = registry.registered_types();
    assert!(types.contains(&ProviderType::Ollama));
    assert!(types.contains(&ProviderType::OpenAI));
    assert!(types.contains(&ProviderType::Anthropic));
}

#[tokio::test]
async fn test_ollama_provider_creation() {
    let registry = ProviderFactoryRegistry::new();

    let config = ProviderConfig {
        provider_type: ProviderType::Ollama,
        base_url: "http://localhost:11434".to_string(),
        auth_ref: None,
        default_model: Some("llama3.2:latest".to_string()),
        timeout_secs: Some(30),
        max_retries: Some(3),
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };

    let result = registry.create_provider(&config).await;
    assert!(result.is_ok());

    let provider = result.unwrap();
    assert_eq!(provider.name(), "ollama");
}

#[tokio::test]
async fn test_openai_provider_validation() {
    let registry = ProviderFactoryRegistry::new();

    // Test without API key
    let config = ProviderConfig {
        provider_type: ProviderType::OpenAI,
        base_url: "https://api.openai.com/v1".to_string(),
        auth_ref: None,
        default_model: Some("gpt-4".to_string()),
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };

    let factory = registry.get_factory(&ProviderType::OpenAI).unwrap();
    let result = factory.validate_config(&config).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("API key required"));
}

#[tokio::test]
async fn test_anthropic_provider_validation() {
    let registry = ProviderFactoryRegistry::new();

    // Test with invalid URL scheme
    let config = ProviderConfig {
        provider_type: ProviderType::Anthropic,
        base_url: "ftp://api.anthropic.com".to_string(),
        auth_ref: Some("ANTHROPIC_API_KEY".to_string()),
        default_model: None,
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };

    let factory = registry.get_factory(&ProviderType::Anthropic).unwrap();
    let result = factory.validate_config(&config).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid URL scheme")
    );
}

#[tokio::test]
#[cfg(feature = "memory")]
async fn test_auth_manager_encryption() {
    let auth_manager = AuthManager::new().await.unwrap();

    // Store a credential
    let credential_id = auth_manager
        .store_credential(
            "test-provider",
            AuthMethod::ApiKey,
            "secret-api-key-12345",
            Some("Test API key".to_string()),
        )
        .await
        .unwrap();

    // Retrieve the credential
    let retrieved = auth_manager.get_credential(&credential_id).await.unwrap();
    assert_eq!(retrieved.value, "secret-api-key-12345");
    assert_eq!(retrieved.metadata.provider_name, "test-provider");

    // Clean up
    auth_manager
        .remove_credential(&credential_id)
        .await
        .unwrap();
}

#[tokio::test]
#[cfg(feature = "memory")]
async fn test_auth_manager_different_auth_methods() {
    let auth_manager = AuthManager::new().await.unwrap();

    // Test Bearer Token
    let bearer_id = auth_manager
        .store_credential(
            "oauth-provider",
            AuthMethod::BearerToken,
            "bearer-token-xyz",
            None,
        )
        .await
        .unwrap();

    let headers =
        arkavo_dataflow::nodes::auth_manager::create_auth_headers(&auth_manager, &bearer_id)
            .await
            .unwrap();
    assert_eq!(
        headers.get("Authorization").unwrap(),
        "Bearer bearer-token-xyz"
    );

    // Test Custom header
    let custom_id = auth_manager
        .store_credential(
            "custom-provider",
            AuthMethod::Custom("X-Custom-Auth".to_string()),
            "custom-value",
            None,
        )
        .await
        .unwrap();

    let headers =
        arkavo_dataflow::nodes::auth_manager::create_auth_headers(&auth_manager, &custom_id)
            .await
            .unwrap();
    assert_eq!(headers.get("X-Custom-Auth").unwrap(), "custom-value");

    // Clean up
    auth_manager.remove_credential(&bearer_id).await.unwrap();
    auth_manager.remove_credential(&custom_id).await.unwrap();
}

#[tokio::test]
async fn test_mock_provider() {
    use arkavo_llm::Provider;
    use common::mock_provider::MockProvider;

    let mock = MockProvider::new(vec![
        "First response".to_string(),
        "Second response".to_string(),
    ]);

    let messages = vec![Message {
        role: Role::User,
        content: "Test message".to_string(),
        images: None,
        tool_call_id: None,
        tool_name: None,
        tool_calls: Vec::new(),
    }];

    let response1 = mock.complete(messages.clone()).await.unwrap();
    assert_eq!(response1, "First response");

    let response2 = mock.complete(messages).await.unwrap();
    assert_eq!(response2, "Second response");

    assert_eq!(*mock.request_count.read().await, 2);
}

#[tokio::test]
async fn test_provider_error_handling() {
    use arkavo_llm::common::ProviderError;
    use std::time::Duration;

    // Test rate limit error
    let error = ProviderError::RateLimited {
        retry_after: Some(Duration::from_mins(1)),
        message: Some("Too many requests".to_string()),
    };
    assert!(error.is_retryable());
    assert_eq!(error.retry_delay(), Some(Duration::from_mins(1)));

    // Test authentication error
    let error = ProviderError::AuthenticationFailed {
        message: "Invalid API key".to_string(),
        provider: "openai".to_string(),
    };
    assert!(!error.is_retryable());
    assert!(error.is_auth_error());

    // Test model not found
    let error = ProviderError::ModelNotFound {
        model: "gpt-5".to_string(),
        provider: "openai".to_string(),
        available_models: Some(vec!["gpt-4".to_string(), "gpt-3.5-turbo".to_string()]),
    };
    assert!(!error.is_retryable());
    assert!(error.is_client_error());
}

#[tokio::test]
async fn test_http_client_retry() {
    use arkavo_llm::common::{HttpClientBuilder, HttpClientConfig};

    let config = HttpClientConfig {
        initial_retry_delay_ms: 100,
        backoff_factor: 2.0,
        max_retry_delay_ms: 1000,
        jitter_factor: 0.0, // No jitter for predictable testing
        ..Default::default()
    };

    let builder = HttpClientBuilder::new(config);

    // Test exponential backoff
    assert_eq!(builder.calculate_retry_delay(0).as_millis(), 100);
    assert_eq!(builder.calculate_retry_delay(1).as_millis(), 200);
    assert_eq!(builder.calculate_retry_delay(2).as_millis(), 400);
    assert_eq!(builder.calculate_retry_delay(3).as_millis(), 800);
    assert_eq!(builder.calculate_retry_delay(4).as_millis(), 1000); // Capped at max
}
