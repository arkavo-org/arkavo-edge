use arkavo_dataflow::nodes::llm_config::{LlmConfigBuilder, LlmConfiguration, LlmProviderConfig};

#[test]
fn test_llm_configuration_new() {
    let config = LlmConfiguration::new();
    assert_eq!(config.providers.len(), 1);
    assert_eq!(config.providers[0].name, "local-ollama");
    assert_eq!(config.providers[0].base_url, "http://localhost:11434");
    assert!(config.model_preferences.is_empty());
}

#[test]
fn test_add_provider() {
    let mut config = LlmConfiguration::new();

    let remote_provider = LlmProviderConfig {
        name: "remote-ollama".to_string(),
        base_url: "http://10.0.0.101:11434".to_string(),
        default_model: Some("llama3.2:latest".to_string()),
        auth_ref: None,
        description: Some("Remote Ollama instance".to_string()),
    };

    config.add_provider(remote_provider);
    assert_eq!(config.providers.len(), 2);
    assert_eq!(config.providers[1].name, "remote-ollama");
    assert_eq!(config.providers[1].base_url, "http://10.0.0.101:11434");
}

#[test]
fn test_model_preferences() {
    let mut config = LlmConfiguration::new();

    // Set preferences
    config.set_model_preference("code_review".to_string(), "devstral:latest".to_string());
    config.set_model_preference("summarization".to_string(), "llama3.2:latest".to_string());

    assert_eq!(config.model_preferences.len(), 2);
    assert_eq!(
        config.model_preferences.get("code_review").unwrap(),
        "devstral:latest"
    );
    assert_eq!(
        config.model_preferences.get("summarization").unwrap(),
        "llama3.2:latest"
    );

    // Update preference
    config.set_model_preference("code_review".to_string(), "codellama:latest".to_string());
    assert_eq!(
        config.model_preferences.get("code_review").unwrap(),
        "codellama:latest"
    );
}

#[test]
fn test_config_builder() {
    let mut builder = LlmConfigBuilder::new();

    // Add remote provider
    builder
        .add_remote_ollama("http://192.168.1.100:11434", Some("edge-box"))
        .unwrap();

    // Set task models
    builder.set_task_model("translation", "qwen3:latest");
    builder.set_task_model("code_review", "devstral:latest");

    let config = builder.build();

    // Should have 2 providers (local + remote)
    assert_eq!(config.providers.len(), 2);

    // Check remote provider
    let remote = config
        .providers
        .iter()
        .find(|p| p.name == "edge-box")
        .unwrap();
    assert_eq!(remote.base_url, "http://192.168.1.100:11434");

    // Check model preferences
    assert_eq!(config.model_preferences.len(), 2);
    assert_eq!(
        config.model_preferences.get("translation").unwrap(),
        "qwen3:latest"
    );
}

#[test]
fn test_config_builder_with_existing() {
    let existing_config = LlmConfiguration::new();
    let builder = LlmConfigBuilder::with_existing(existing_config.clone());

    let config = builder.build();
    assert_eq!(config.providers.len(), existing_config.providers.len());
}

#[test]
fn test_provider_config_serialization() {
    use serde_json;

    let provider = LlmProviderConfig {
        name: "test-provider".to_string(),
        base_url: "http://test.com:11434".to_string(),
        default_model: Some("test-model".to_string()),
        auth_ref: Some("auth-123".to_string()),
        description: Some("Test provider".to_string()),
    };

    // Serialize
    let json = serde_json::to_string(&provider).unwrap();
    assert!(json.contains("test-provider"));
    assert!(json.contains("http://test.com:11434"));

    // Deserialize
    let deserialized: LlmProviderConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.name, provider.name);
    assert_eq!(deserialized.base_url, provider.base_url);
    assert_eq!(deserialized.default_model, provider.default_model);
    assert_eq!(deserialized.auth_ref, provider.auth_ref);
}

#[test]
fn test_configuration_serialization() {
    use serde_json;

    let mut config = LlmConfiguration::new();
    config.set_model_preference("test".to_string(), "model".to_string());

    // Serialize
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("local-ollama"));
    assert!(json.contains("model_preferences"));

    // Deserialize
    let deserialized: LlmConfiguration = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.providers.len(), config.providers.len());
    assert_eq!(deserialized.model_preferences.get("test").unwrap(), "model");
}

#[tokio::test]
async fn test_remote_ollama_url_validation() {
    let mut builder = LlmConfigBuilder::new();

    // Valid URLs
    assert!(
        builder
            .add_remote_ollama("http://192.168.1.100:11434", None)
            .is_ok()
    );
    assert!(
        builder
            .add_remote_ollama("https://api.ollama.com", Some("cloud"))
            .is_ok()
    );

    // Note: The current implementation doesn't validate URLs in add_remote_ollama
    // This test documents the current behavior
}

#[test]
fn test_updated_timestamp() {
    let config1 = LlmConfiguration::new();
    let initial_time = config1.updated_at;

    // Sleep a tiny bit to ensure time difference
    std::thread::sleep(std::time::Duration::from_millis(10));

    let mut config2 = LlmConfiguration::new();
    config2.add_provider(LlmProviderConfig {
        name: "new-provider".to_string(),
        base_url: "http://example.com".to_string(),
        default_model: None,
        auth_ref: None,
        description: None,
    });

    // Updated time should be different
    assert!(config2.updated_at > initial_time);
}
