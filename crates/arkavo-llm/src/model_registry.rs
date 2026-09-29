//! Model Registry - Multi-Model Concurrent Inference
//!
//! Manages multiple loaded llama.cpp models in the same process and handles
//! concurrent inference requests to different models simultaneously.
//!
//! Architecture:
//! - Each model is stored as Arc<LlamaModel> for thread-safe shared access
//! - Each model has a bounded ContextPool; requests for a busy model queue
//! - Different models run concurrently, each on its own context
//! - KV cache isolation: each context has its own cache for conversations

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::LlamaModel;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::multimodal::MtmdContext;
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

/// Context window of every model loaded in this process, by registry name.
///
/// Process-wide rather than per registry because the planner that needs the
/// figure is handed a model name, not the registry that loaded it.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
static CONTEXT_LENGTHS: std::sync::LazyLock<RwLock<HashMap<String, u32>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// The context window a loaded model's contexts are created with, or `None`
/// when no registry in this process has loaded a model under `name`.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub fn loaded_context_length(name: &str) -> Option<u32> {
    CONTEXT_LENGTHS
        .read()
        .ok()
        .and_then(|lengths| lengths.get(name).copied())
}

/// No model is ever loaded in a build without the local engine.
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub fn loaded_context_length(_name: &str) -> Option<u32> {
    None
}

/// A pooled context that returns to its pool when dropped.
///
/// Returning on drop rather than by an explicit call matters once the pool
/// is a hard bound: a generation task that panics or is cancelled would
/// otherwise keep the model's only context forever.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct ContextLease {
    registry: Arc<ModelRegistry>,
    model_name: String,
    context: Option<PooledContext>,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl ContextLease {
    /// The leased context, for the duration of one generation.
    pub fn context(&self) -> Option<Arc<std::sync::Mutex<arkavo_llama_cpp::LlamaContext>>> {
        self.context.as_ref().map(|pooled| pooled.context.clone())
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl Drop for ContextLease {
    fn drop(&mut self) {
        if let Some(context) = self.context.take()
            && let Err(e) = self
                .registry
                .release_context(&self.model_name, context, true)
        {
            tracing::warn!(model = %self.model_name, error = %e, "Context was not returned to its pool");
        }
    }
}

/// Registry for managing multiple loaded models with pooled contexts
///
/// The registry stores loaded models and uses a ContextPool for managing
/// multiple contexts per model, enabling true concurrent inference.
pub struct ModelRegistry {
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    models: RwLock<HashMap<String, Arc<LlamaModel>>>,
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    context_pool: ContextPool,
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    vision_contexts: RwLock<HashMap<String, Arc<MtmdContext>>>,
    // Stub fields for non-llama-cpp builds to maintain struct size consistency
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    models: RwLock<HashSet<String>>,
}

impl ModelRegistry {
    /// Create a new empty model registry with default pool settings
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn new() -> Self {
        Self::with_max_contexts(crate::context_pool::default_max_contexts())
    }

    /// Create a new model registry with custom max contexts per model
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn with_max_contexts(max_contexts: usize) -> Self {
        Self {
            models: RwLock::new(HashMap::new()),
            context_pool: ContextPool::with_max_contexts(max_contexts),
            vision_contexts: RwLock::new(HashMap::new()),
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
        // Double-check under read lock to avoid concurrent duplicate loads
        if self.is_loaded(name) {
            return Ok(());
        }

        if crate::gguf_tdf::is_protected_model_path(path) {
            return Err(Error::Config(format!(
                "GGUFTDF_KAS_DENIED: {path} is a protected model and needs a \
                 KAS rewrap; run 'arkavo login' then retry"
            )));
        }

        let model = LlamaModel::from_file(path)
            .map_err(|e| Error::Config(format!("Failed to load model from {path}: {e}")))?;

        self.register_loaded(name, model)
    }

    /// Load a KAS-protected `.gguf.tdf` model with an already-recovered
    /// payload key and register it.
    ///
    /// KAS rewrap is asynchronous and this method is not, so the caller
    /// performs the round-trip in the runtime it already owns and passes the
    /// key in. Nothing here contacts a KAS or falls back to a plaintext model.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn load_protected(&self, name: &str, path: &str, payload_key: [u8; 32]) -> Result<()> {
        if self.is_loaded(name) {
            return Ok(());
        }

        let model = crate::gguf_tdf::load_with_payload_key(path, payload_key)?;
        self.register_loaded(name, model)
    }

    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    fn register_loaded(&self, name: &str, model: LlamaModel) -> Result<()> {
        let model_arc = Arc::new(model);

        {
            let mut models = self
                .models
                .write()
                .map_err(|_| Error::Internal("Lock poisoned".to_string()))?;
            // Final check under write lock — another thread may have loaded it
            if models.contains_key(name) {
                return Ok(());
            }
            // Register with context pool for concurrent context management
            self.context_pool.register_model(name, model_arc.clone())?;
            let n_ctx =
                arkavo_llama_cpp::configured_context_length(model_arc.get_trained_context_size());
            if let Ok(mut lengths) = CONTEXT_LENGTHS.write() {
                lengths.insert(name.to_string(), n_ctx);
            }
            tracing::info!(
                model = name,
                n_ctx,
                max_contexts = self.context_pool.max_contexts(),
                "Model registered"
            );
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

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn load_protected(&self, _name: &str, _path: &str, _payload_key: [u8; 32]) -> Result<()> {
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

    /// Lease a fresh context, waiting up to `limit` for one to be released
    /// when every context for the model is in use.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub async fn lease_fresh_context(
        self: &Arc<Self>,
        name: &str,
        limit: std::time::Duration,
    ) -> Result<ContextLease> {
        let context = self.context_pool.acquire_fresh_within(name, limit).await?;
        Ok(ContextLease {
            registry: Arc::clone(self),
            model_name: name.to_string(),
            context: Some(context),
        })
    }

    /// Get a cached vision context for a model, if one has been stored.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn get_vision_ctx(&self, name: &str) -> Option<Arc<MtmdContext>> {
        self.vision_contexts
            .read()
            .ok()
            .and_then(|ctxs| ctxs.get(name).cloned())
    }

    /// Store a vision context for a model so subsequent provider creations skip the load.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn store_vision_ctx(&self, name: &str, ctx: Arc<MtmdContext>) {
        if let Ok(mut ctxs) = self.vision_contexts.write() {
            ctxs.insert(name.to_string(), ctx);
        }
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
            let removed = self
                .models
                .write()
                .ok()
                .and_then(|mut models| models.remove(name))
                .is_some();
            if removed && let Ok(mut lengths) = CONTEXT_LENGTHS.write() {
                lengths.remove(name);
            }
            removed
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
        let registry = ModelRegistry::default();
        assert!(registry.is_empty());
    }

    /// Test thread safety - ModelRegistry should be Send + Sync
    #[test]
    fn test_registry_thread_safety() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ModelRegistry>();
    }
}
