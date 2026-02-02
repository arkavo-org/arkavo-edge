//! Model Registry - Multi-Model Concurrent Inference
//!
//! Manages multiple loaded llama.cpp models in the same process and handles
//! concurrent inference requests to different models simultaneously.
//!
//! Architecture:
//! - Each model is stored as Arc<LlamaModel> for thread-safe shared access
//! - Each model gets its own Mutex<LlamaContext> because llama_decode() is not reentrant
//! - Concurrent requests to the same model queue on the mutex
//! - Requests to different models run truly in parallel

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::{LlamaContext, LlamaModel};
use std::collections::HashMap;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Mutex;
use std::sync::{Arc, RwLock};

use crate::{Error, Result};

/// Guard for exclusive access to a model's context
///
/// When dropped, the context is returned to the pool for other requests.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct ContextGuard {
    pub context: Arc<Mutex<LlamaContext>>,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl ContextGuard {
    fn new(context: Arc<Mutex<LlamaContext>>) -> Self {
        Self { context }
    }
}

/// Registry for managing multiple loaded models
///
/// The registry stores loaded models and their associated contexts,
/// allowing concurrent access to different models while ensuring
/// thread-safe access to individual models.
pub struct ModelRegistry {
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    models: RwLock<HashMap<String, Arc<LlamaModel>>>,
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    contexts: RwLock<HashMap<String, Arc<Mutex<LlamaContext>>>>,
    // Stub fields for non-llama-cpp builds to maintain struct size consistency
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    models: RwLock<HashMap<String, ()>>,
}

impl ModelRegistry {
    /// Create a new empty model registry
    pub fn new() -> Self {
        Self {
            models: RwLock::new(HashMap::new()),
            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            contexts: RwLock::new(HashMap::new()),
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

        let context = LlamaContext::new(&model)
            .map_err(|e| Error::Config(format!("Failed to create context: {e}")))?;

        {
            let mut models = self
                .models
                .write()
                .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;
            models.insert(name.to_string(), Arc::new(model));
        }

        {
            let mut contexts = self
                .contexts
                .write()
                .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;
            contexts.insert(name.to_string(), Arc::new(Mutex::new(context)));
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

    /// Acquire exclusive access to a model's context
    ///
    /// This blocks until the context is available. Multiple requests to the
    /// same model will be serialized; requests to different models run in parallel.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    #[allow(clippy::significant_drop_tightening)]
    pub fn acquire_context(&self, name: &str) -> Result<ContextGuard> {
        let context = self
            .contexts
            .read()
            .map_err(|_| Error::Internal("Lock poisoned".to_string()))?
            .get(name)
            .cloned()
            .ok_or_else(|| Error::Config(format!("Model '{name}' not found in registry")))?;

        Ok(ContextGuard::new(context))
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
            let removed = self
                .models
                .write()
                .ok()
                .and_then(|mut models| models.remove(name))
                .is_some();
            if let Ok(mut contexts) = self.contexts.write() {
                contexts.remove(name);
            }
            removed
        }
        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            false
        }
    }

    /// Check if a model is currently loaded in the registry
    pub fn is_loaded(&self, name: &str) -> bool {
        self.models
            .read()
            .ok()
            .map(|models| models.contains_key(name))
            .unwrap_or(false)
    }

    /// Get a list of all loaded model names
    pub fn model_names(&self) -> Vec<String> {
        self.models
            .read()
            .ok()
            .map(|models| models.keys().cloned().collect())
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
