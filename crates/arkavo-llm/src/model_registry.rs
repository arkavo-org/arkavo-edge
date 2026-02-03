//! Model Registry - Multi-Model Concurrent Inference
//!
//! Manages multiple loaded llama.cpp models in the same process and handles
//! concurrent inference requests to different models simultaneously.
//!
//! Architecture:
//! - Each model is stored as Arc<LlamaModel> for thread-safe shared access
//! - Models use ContextPool for multiple concurrent contexts per model
//! - True concurrent inference: different contexts = parallel execution
//! - KV cache isolation: each context has its own cache for conversations

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::LlamaModel;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::collections::HashMap;
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
use std::collections::HashSet;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Arc;
use std::sync::RwLock;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use crate::context_pool::{ContextPool, PooledContext};
use crate::{Error, Result};

/// Registry for managing multiple loaded models with pooled contexts
///
/// The registry stores loaded models and uses a ContextPool for managing
/// multiple contexts per model, enabling true concurrent inference.
pub struct ModelRegistry {
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    models: RwLock<HashMap<String, Arc<LlamaModel>>>,
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    context_pool: ContextPool,
    // Stub fields for non-llama-cpp builds to maintain struct size consistency
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    models: RwLock<HashSet<String>>,
}

impl ModelRegistry {
    /// Create a new empty model registry with default pool settings
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn new() -> Self {
        Self::with_max_contexts(4)
    }

    /// Create a new model registry with custom max contexts per model
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn with_max_contexts(max_contexts: usize) -> Self {
        Self {
            models: RwLock::new(HashMap::new()),
            context_pool: ContextPool::with_max_contexts(max_contexts),
        }
    }

    /// Create a new empty model registry (stub for non-llama-cpp builds)
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn new() -> Self {
        Self {
            models: RwLock::new(HashSet::new()),
        }
    }

    /// Load a model from a file path and register it with the given name
    ///
    /// # Arguments
    /// * `name` - Unique identifier for this model in the registry
    /// * `path` - File system path to the GGUF model file
    ///
    /// # Errors
    /// Returns an error if the model fails to load from the given path
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn load(&self, name: &str, path: &str) -> Result<()> {
        let model = LlamaModel::from_file(path)
            .map_err(|e| Error::Config(format!("Failed to load model from {path}: {e}")))?;

        let model_arc = Arc::new(model);

        // Register with context pool for concurrent context management
        self.context_pool.register_model(name, model_arc.clone())?;

        {
            let mut models = self
                .models
                .write()
                .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;
            models.insert(name.to_string(), model_arc);
        }

        Ok(())
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn load(&self, _name: &str, _path: &str) -> Result<()> {
        Err(Error::Config(
            "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
        ))
    }

    /// Get a reference to a loaded model by name
    ///
    /// Returns None if the model is not loaded
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn get(&self, name: &str) -> Option<Arc<LlamaModel>> {
        self.models
            .read()
            .ok()
            .and_then(|models| models.get(name).cloned())
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn get(&self, _name: &str) -> Option<()> {
        None
    }

    /// Acquire a context from the pool for the given model
    ///
    /// Returns a PooledContext that can be used for inference. The context
    /// preserves its KV cache for multi-turn conversations.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn acquire_context(&self, name: &str) -> Result<PooledContext> {
        self.context_pool.acquire(name)
    }

    /// Acquire a fresh context with cleared KV cache (for new conversations)
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn acquire_fresh_context(&self, name: &str) -> Result<PooledContext> {
        self.context_pool.acquire_fresh(name)
    }

    /// Release a context back to the pool
    ///
    /// # Arguments
    /// * `name` - Model name
    /// * `context` - The context to release
    /// * `clear_cache` - If true, clears KV cache before returning to pool
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn release_context(
        &self,
        name: &str,
        context: PooledContext,
        clear_cache: bool,
    ) -> Result<()> {
        self.context_pool.release(name, context, clear_cache)
    }

    /// Get the context pool (for advanced use cases like ConversationContextManager)
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn context_pool(&self) -> &ContextPool {
        &self.context_pool
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn acquire_context(&self, name: &str) -> Result<()> {
        Err(Error::Config(format!(
            "Model '{name}' not found (llama-cpp not enabled)"
        )))
    }

    /// Unload a model from the registry, freeing its resources
    ///
    /// Returns true if a model was removed, false if it wasn't loaded
    pub fn unload_model(&self, name: &str) -> bool {
        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        {
            // Remove from models map (contexts will be cleaned up when pool is dropped)
            self.models
                .write()
                .ok()
                .and_then(|mut models| models.remove(name))
                .is_some()
        }
        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            let _ = name;
            false
        }
    }

    /// Check if a model is currently loaded in the registry
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn is_loaded(&self, name: &str) -> bool {
        self.models
            .read()
            .ok()
            .map(|models| models.contains_key(name))
            .unwrap_or(false)
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn is_loaded(&self, name: &str) -> bool {
        self.models
            .read()
            .ok()
            .map(|models| models.contains(name))
            .unwrap_or(false)
    }

    /// Get a list of all loaded model names
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn model_names(&self) -> Vec<String> {
        self.models
            .read()
            .ok()
            .map(|models| models.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn model_names(&self) -> Vec<String> {
        self.models
            .read()
            .ok()
            .map(|models| models.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get the number of loaded models
    pub fn len(&self) -> usize {
        self.models
            .read()
            .ok()
            .map(|models| models.len())
            .unwrap_or(0)
    }

    /// Check if the registry has no loaded models
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// List all models with their information
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn list_models(&self) -> Vec<ModelInfo> {
        self.models
            .read()
            .ok()
            .map(|models| {
                models
                    .keys()
                    .map(|name| ModelInfo {
                        name: name.clone(),
                        loaded: true,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn list_models(&self) -> Vec<ModelInfo> {
        self.models
            .read()
            .ok()
            .map(|models| {
                models
                    .iter()
                    .map(|name| ModelInfo {
                        name: name.clone(),
                        loaded: true,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a loaded model
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub loaded: bool,
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = ModelRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_registry_is_loaded_empty() {
        let registry = ModelRegistry::new();
        assert!(!registry.is_loaded("any-model"));
    }

    #[test]
    fn test_registry_model_names_empty() {
        let registry = ModelRegistry::new();
        let names = registry.model_names();
        assert!(names.is_empty());
    }

    #[test]
    fn test_registry_list_models_empty() {
        let registry = ModelRegistry::new();
        let models = registry.list_models();
        assert!(models.is_empty());
    }

    #[test]
    fn test_registry_unload_nonexistent() {
        let registry = ModelRegistry::new();
        assert!(!registry.unload_model("non-existent"));
    }

    #[test]
    fn test_registry_default() {
        let registry: ModelRegistry = Default::default();
        assert!(registry.is_empty());
    }

    /// Test thread safety - ModelRegistry should be Send + Sync
    #[test]
    fn test_registry_thread_safety() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ModelRegistry>();
    }
}
