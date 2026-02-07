#![allow(clippy::disallowed_methods)]
#![allow(unexpected_cfgs)]
#![allow(unused_variables)]
#![allow(clippy::float_cmp)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::needless_collect)]

use arkavo_dataflow::nodes::model_registry::{ModelCapabilities, ModelInfo, get_default_models};
use chrono::Utc;

#[tokio::test]
async fn test_default_models() {
    let models = get_default_models();
    assert!(!models.is_empty());

    // Check for expected models
    let model_ids: Vec<String> = models.iter().map(|m| m.model_id.clone()).collect();
    assert!(model_ids.contains(&"llama3.2:latest".to_string()));
    assert!(model_ids.contains(&"devstral:latest".to_string()));
    assert!(model_ids.contains(&"qwen3:0.6b".to_string()));

    // Check model properties
    let llama = models
        .iter()
        .find(|m| m.model_id == "llama3.2:latest")
        .unwrap();
    assert_eq!(llama.provider_name, "ollama");
    assert!(llama.capabilities.supports_streaming);
}

#[test]
fn test_model_capabilities_default() {
    let caps = ModelCapabilities::default();
    assert!(caps.supports_streaming);
    assert!(!caps.supports_function_calling);
    assert!(!caps.supports_vision);
    assert!(!caps.supports_code_execution);
    assert_eq!(caps.input_modalities, vec!["text"]);
    assert_eq!(caps.output_modalities, vec!["text"]);
    assert_eq!(caps.languages, vec!["en"]);
    assert!(caps.specializations.is_empty());
}

#[test]
fn test_model_info_serialization() {
    use serde_json;

    let model = ModelInfo {
        model_id: "test-model".to_string(),
        provider_name: "test-provider".to_string(),
        display_name: "Test Model".to_string(),
        description: Some("A test model".to_string()),
        context_window: Some(4096),
        max_output_tokens: Some(2048),
        capabilities: ModelCapabilities::default(),
        tags: vec!["test".to_string(), "demo".to_string()],
        version: Some("1.0".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: None,
        pricing: None,
        snpe_metadata: None,
    };

    // Test serialization
    let json = serde_json::to_string(&model).unwrap();
    assert!(json.contains("test-model"));
    assert!(json.contains("test-provider"));

    // Test deserialization
    let deserialized: ModelInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.model_id, model.model_id);
    assert_eq!(deserialized.provider_name, model.provider_name);
    assert_eq!(deserialized.tags, model.tags);
}

#[tokio::test]
async fn test_model_search_by_capability() {
    // Create a test model with specific capabilities
    let code_model = ModelInfo {
        model_id: "code-model".to_string(),
        provider_name: "test".to_string(),
        display_name: "Code Model".to_string(),
        description: None,
        context_window: Some(8192),
        max_output_tokens: Some(4096),
        capabilities: ModelCapabilities {
            supports_streaming: true,
            supports_function_calling: true,
            supports_vision: false,
            supports_code_execution: true,
            input_modalities: vec!["text".to_string()],
            output_modalities: vec!["text".to_string()],
            languages: vec!["en".to_string()],
            specializations: vec!["code".to_string()],
        },
        tags: vec!["code".to_string()],
        version: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: None,
        pricing: None,
        snpe_metadata: None,
    };

    let vision_model = ModelInfo {
        model_id: "vision-model".to_string(),
        provider_name: "test".to_string(),
        display_name: "Vision Model".to_string(),
        description: None,
        context_window: Some(4096),
        max_output_tokens: Some(2048),
        capabilities: ModelCapabilities {
            supports_streaming: true,
            supports_function_calling: false,
            supports_vision: true,
            supports_code_execution: false,
            input_modalities: vec!["text".to_string(), "image".to_string()],
            output_modalities: vec!["text".to_string()],
            languages: vec!["en".to_string()],
            specializations: vec![],
        },
        tags: vec!["vision".to_string()],
        version: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: None,
        pricing: None,
        snpe_metadata: None,
    };

    // Test capability filtering
    let models = [code_model, vision_model];

    let streaming_models: Vec<_> = models
        .iter()
        .filter(|m| m.capabilities.supports_streaming)
        .collect();
    assert_eq!(streaming_models.len(), 2);

    let function_calling_models: Vec<_> = models
        .iter()
        .filter(|m| m.capabilities.supports_function_calling)
        .collect();
    assert_eq!(function_calling_models.len(), 1);
    assert_eq!(function_calling_models[0].model_id, "code-model");

    let vision_models: Vec<_> = models
        .iter()
        .filter(|m| m.capabilities.supports_vision)
        .collect();
    assert_eq!(vision_models.len(), 1);
    assert_eq!(vision_models[0].model_id, "vision-model");
}

#[tokio::test]
async fn test_find_models_for_task() {
    let models = get_default_models();

    // Find code review models
    let code_models: Vec<_> = models
        .iter()
        .filter(|m| {
            m.tags.iter().any(|t| t == "code" || t == "programming")
                || m.capabilities.specializations.contains(&"code".to_string())
        })
        .collect();
    assert!(!code_models.is_empty());

    // Find summarization models
    let summary_models: Vec<_> = models
        .iter()
        .filter(|m| {
            m.tags
                .iter()
                .any(|t| t == "summarization" || t == "general")
        })
        .collect();
    assert!(!summary_models.is_empty());

    // Find translation models
    let _translation_models: Vec<_> = models
        .iter()
        .filter(|m| {
            m.tags
                .iter()
                .any(|t| t == "translation" || t == "multilingual")
                || m.capabilities.languages.len() > 1
        })
        .collect();
    // May or may not have translation models in defaults
}

#[tokio::test]
async fn test_model_export_import() {
    use serde_json;

    let models = get_default_models();

    // Create export data
    let export = serde_json::json!({
        "version": "1.0",
        "exported_at": Utc::now().to_rfc3339(),
        "model_count": models.len(),
        "models": models
    });

    let export_json = serde_json::to_string_pretty(&export).unwrap();
    assert!(export_json.contains("version"));
    assert!(export_json.contains("models"));

    // Test import parsing
    let import_data: serde_json::Value = serde_json::from_str(&export_json).unwrap();
    assert_eq!(import_data["version"], "1.0");

    let imported_models = import_data["models"].as_array().unwrap();
    assert_eq!(imported_models.len(), models.len());
}
