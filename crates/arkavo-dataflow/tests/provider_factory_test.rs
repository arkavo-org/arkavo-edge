#![allow(clippy::disallowed_methods)]
#![allow(unexpected_cfgs)]
#![allow(unused_variables)]
#![allow(clippy::float_cmp)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::needless_collect)]

use arkavo_llm::providers::{
    OllamaProviderFactory, ProviderConfig, ProviderFactory, ProviderFactoryRegistry, ProviderType,
};

#[tokio::test]
async fn test_provider_type_detection() {
    assert_eq!(
        ProviderType::from_name("local-ollama"),
        ProviderType::Ollama
    );
    assert_eq!(
        ProviderType::from_name("remote-ollama"),
        ProviderType::Ollama
    );
    assert_eq!(ProviderType::from_name("openai-gpt4"), ProviderType::OpenAI);
    assert_eq!(
        ProviderType::from_name("custom-provider"),
        ProviderType::Custom("custom-provider".to_string())
    );
}

#[tokio::test]
async fn test_ollama_factory_validation() {
    let factory = OllamaProviderFactory;

    // Test empty URL
    let config = ProviderConfig {
        provider_type: ProviderType::Ollama,
        base_url: String::new(),
        auth_ref: None,
        default_model: None,
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };
    assert!(factory.validate_config(&config).await.is_err());

    // Test invalid URL
    let config = ProviderConfig {
        provider_type: ProviderType::Ollama,
        base_url: "not-a-url".to_string(),
        auth_ref: None,
        default_model: None,
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };
    assert!(factory.validate_config(&config).await.is_err());

    // Test non-HTTP scheme
    let config = ProviderConfig {
        provider_type: ProviderType::Ollama,
        base_url: "ftp://localhost:11434".to_string(),
        auth_ref: None,
        default_model: None,
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };
    assert!(factory.validate_config(&config).await.is_err());

    // Test valid URL
    let config = ProviderConfig {
        provider_type: ProviderType::Ollama,
        base_url: "http://localhost:11434".to_string(),
        auth_ref: None,
        default_model: None,
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };
    assert!(factory.validate_config(&config).await.is_ok());

    // Test HTTPS URL
    let config = ProviderConfig {
        provider_type: ProviderType::Ollama,
        base_url: "https://api.ollama.com".to_string(),
        auth_ref: None,
        default_model: None,
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };
    assert!(factory.validate_config(&config).await.is_ok());
}

#[tokio::test]
async fn test_provider_factory_registry() {
    let registry = ProviderFactoryRegistry::new();

    // Check that all factories are registered by default
    assert!(registry.get_factory(&ProviderType::Ollama).is_some());
    assert!(registry.get_factory(&ProviderType::OpenAI).is_some());
    assert!(registry.get_factory(&ProviderType::Anthropic).is_some());

    // Test registered types - check for required providers
    let types = registry.registered_types();
    assert!(types.contains(&ProviderType::Ollama));
    assert!(types.contains(&ProviderType::OpenAI));
    assert!(types.contains(&ProviderType::Anthropic));

    // Note: Due to feature unification in workspace builds, additional providers
    // (Local, DeepSeek, etc.) may be registered even if not directly enabled.
    // This is expected behavior when other crates in the workspace enable those features.
    // We just verify the minimum required providers are present.
    assert!(
        types.len() >= 3,
        "Expected at least 3 providers (Ollama, OpenAI, Anthropic), got {}",
        types.len()
    );
}

#[tokio::test]
#[ignore = "Memory alignment issue with auth manager"]
async fn test_provider_creation() {
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

    // Should successfully create an Ollama provider
    let result = registry.create_provider(&config).await;
    assert!(result.is_ok());

    // Test with OpenAI provider (should fail due to missing auth credentials)
    let config = ProviderConfig {
        provider_type: ProviderType::OpenAI,
        base_url: "https://api.openai.com/v1".to_string(),
        auth_ref: Some("test-api-key".to_string()),
        default_model: Some("gpt-4".to_string()),
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };

    let result = registry.create_provider(&config).await;
    assert!(result.is_err());
    let err = result.err().unwrap();
    println!("Error message: {err}");
    // Should fail because credentials can't be found
    // After sanitization, credential IDs are truncated in error messages
    assert!(
        err.to_string().contains("test-ap...")
            || err.to_string().contains("credential")
            || err.to_string().contains("not found")
    );

    // Test with truly unregistered provider type
    let config = ProviderConfig {
        provider_type: ProviderType::Custom("nonexistent".to_string()),
        base_url: "https://api.example.com".to_string(),
        auth_ref: None,
        default_model: None,
        timeout_secs: None,
        max_retries: None,
        initial_retry_delay_ms: None,
        backoff_factor: None,
        max_retry_delay_ms: None,
        jitter_factor: None,
        metadata: None,
    };

    let result = registry.create_provider(&config).await;
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(err.to_string().contains("No factory registered"));
}

#[test]
fn test_provider_config_serialization() {
    use serde_json;

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

    // Test serialization
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("ollama"));
    assert!(json.contains("http://localhost:11434"));

    // Test deserialization
    let deserialized: ProviderConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.base_url, config.base_url);
    assert_eq!(deserialized.default_model, config.default_model);
}
