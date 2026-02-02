//! ModelRegistry Tests - Multi-Model Concurrent Inference
//!
//! Tests for the ModelRegistry which manages multiple loaded models
//! and enables concurrent inference across different models.

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llm::{ModelRegistry, MultiModelProvider, Provider, SamplingConfig};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Arc;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_model_registry_creation() {
    let registry = ModelRegistry::new();
    assert!(registry.list_models().is_empty());
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_model_registry_load_and_get() {
    let registry = ModelRegistry::new();

    // Initially should have no models
    assert!(registry.get("test-model").is_none());
    assert_eq!(registry.list_models().len(), 0);
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_model_registry_model_names() {
    let registry = ModelRegistry::new();

    // Initially should have no models
    let names = registry.model_names();
    assert!(names.is_empty());
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_model_registry_is_loaded() {
    let registry = ModelRegistry::new();

    // Model should not be loaded initially
    assert!(!registry.is_loaded("any-model"));
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_model_registry_len_and_is_empty() {
    let registry = ModelRegistry::new();

    assert_eq!(registry.len(), 0);
    assert!(registry.is_empty());
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_model_registry_unload_model() {
    let registry = ModelRegistry::new();

    // Unloading a non-existent model should return false
    assert!(!registry.unload_model("non-existent"));
}

/// Test that ModelRegistry is Send + Sync for concurrent access
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[test]
fn test_model_registry_thread_safety() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ModelRegistry>();
}

/// Test Provider trait implementation for multi-model provider
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_multi_model_provider_creation() {
    let registry = Arc::new(ModelRegistry::new());
    let provider = MultiModelProvider::new(registry, "default", SamplingConfig::default());

    assert_eq!(provider.name(), "default");
    assert!(provider.supports_tools());
}
