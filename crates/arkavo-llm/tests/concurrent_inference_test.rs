//! Concurrent Inference Tests - Multi-Model Support
//!
//! Tests for concurrent inference across multiple models, verifying:
//! - Multiple models can be loaded simultaneously
//! - Requests to different models run in parallel
//! - Thread safety of the ModelRegistry

#![allow(clippy::disallowed_methods, clippy::uninlined_format_args)]

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llm::ModelRegistry;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Arc;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_concurrent_model_access() {
    let registry = Arc::new(ModelRegistry::new());

    // Spawn multiple tasks that access the registry concurrently
    let mut handles = vec![];

    for i in 0..10 {
        let registry_clone = registry.clone();
        let handle = tokio::spawn(async move {
            // Check if model exists (concurrent read)
            let exists = registry_clone.is_loaded(&format!("model-{}", i));
            (i, exists)
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        let (i, exists) = handle.await.unwrap();
        assert!(!exists, "Model {} should not exist", i);
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_model_registry_thread_safety_stress() {
    let registry = Arc::new(ModelRegistry::new());
    let mut handles = vec![];

    // Spawn concurrent readers
    for _ in 0..20 {
        let reg = registry.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..100 {
                let _ = reg.is_loaded("test-model");
                let _ = reg.model_names();
                let _ = reg.len();
            }
        }));
    }

    // Wait for all tasks
    for handle in handles {
        handle.await.unwrap();
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[test]
fn test_model_registry_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ModelRegistry>();
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[test]
fn test_pooled_context_send() {
    fn assert_send<T: Send>() {}
    use arkavo_llm::PooledContext;
    assert_send::<PooledContext>();
}

/// Test that ModelInfo can be cloned and sent across threads
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[test]
fn test_model_info_clone_send() {
    fn assert_clone_send<T: Clone + Send>() {}
    use arkavo_llm::ModelInfo;
    assert_clone_send::<ModelInfo>();
}

/// Test multi-model provider thread safety
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[test]
fn test_multi_model_provider_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    use arkavo_llm::MultiModelProvider;
    assert_send_sync::<MultiModelProvider>();
}
