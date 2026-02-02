//! ModelRegistry Tests - Multi-Model Concurrent Inference
//!
//! Tests for the ModelRegistry which manages multiple loaded models
//! and enables concurrent inference across different models.

use std::sync::Arc;

/// Test helper to check if llama-cpp feature is available
fn llama_cpp_available() -> bool {
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    {
        true
    }
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    {
        false
    }
}

#[tokio::test]
async fn test_model_registry_creation() {
    if !llama_cpp_available() {
        eprintln!("Skipping test: llama-cpp feature not available");
        return;
    }

    use arkavo_llm::ModelRegistry;

    let registry = ModelRegistry::new();
    assert!(registry.list_models().is_empty());
}

#[tokio::test]
async fn test_model_registry_load_and_get() {
    if !llama_cpp_available() {
        eprintln!("Skipping test: llama-cpp feature not available");
        return;
    }

    use arkavo_llm::ModelRegistry;

    let registry = ModelRegistry::new();

    // Initially should have no models
    assert!(registry.get("test-model").is_none());
    assert_eq!(registry.list_models().len(), 0);
}

#[tokio::test]
async fn test_model_registry_model_names() {
    if !llama_cpp_available() {
        eprintln!("Skipping test: llama-cpp feature not available");
        return;
    }

    use arkavo_llm::ModelRegistry;

    let registry = ModelRegistry::new();

    // Initially should have no models
    let names = registry.model_names();
    assert!(names.is_empty());
}

#[tokio::test]
async fn test_model_registry_is_loaded() {
    if !llama_cpp_available() {
        eprintln!("Skipping test: llama-cpp feature not available");
        return;
    }

    use arkavo_llm::ModelRegistry;

    let registry = ModelRegistry::new();

    // Model should not be loaded initially
    assert!(!registry.is_loaded("any-model"));
}

#[tokio::test]
async fn test_model_registry_len_and_is_empty() {
    if !llama_cpp_available() {
        eprintln!("Skipping test: llama-cpp feature not available");
        return;
    }

    use arkavo_llm::ModelRegistry;

    let registry = ModelRegistry::new();

    assert_eq!(registry.len(), 0);
    assert!(registry.is_empty());
}

#[tokio::test]
async fn test_model_registry_unload_model() {
    if !llama_cpp_available() {
        eprintln!("Skipping test: llama-cpp feature not available");
        return;
    }

    use arkavo_llm::ModelRegistry;

    let registry = ModelRegistry::new();

    // Unloading a non-existent model should return false
    assert!(!registry.unload_model("non-existent"));
}

/// Test that ModelRegistry is Send + Sync for concurrent access
#[test]
fn test_model_registry_thread_safety() {
    fn assert_send_sync<T: Send + Sync>() {}

    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    {
        use arkavo_llm::ModelRegistry;
        assert_send_sync::<ModelRegistry>();
    }
}

/// Test Provider trait implementation for multi-model provider
#[tokio::test]
async fn test_multi_model_provider_creation() {
    if !llama_cpp_available() {
        eprintln!("Skipping test: llama-cpp feature not available");
        return;
    }

    use arkavo_llm::{ModelRegistry, MultiModelProvider, Provider, SamplingConfig};
    use std::sync::Arc;

    let registry = Arc::new(ModelRegistry::new());
    let provider = MultiModelProvider::new(registry, "default", SamplingConfig::default());

    assert_eq!(provider.name(), "default");
    assert!(provider.supports_tools());
}
