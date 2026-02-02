//! Multi-Model Provider - Concurrent Inference Support
//!
//! Implements the Provider trait for multi-model inference, allowing
//! dynamic model selection per-request while maintaining the same
//! interface as single-model providers.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llama_cpp::multimodal::MtmdContext;

use crate::{
    Error, Message, ModelRegistry, Provider, ProviderResponse, Result, SamplingConfig,
    StreamResponse,
};

/// Provider that can route requests to multiple loaded models
///
/// Uses a ModelRegistry to access different models and can select
/// which model to use based on request parameters or routing logic.
#[allow(dead_code)]
pub struct MultiModelProvider {
    registry: Arc<ModelRegistry>,
    default_model: String,
    config: SamplingConfig,
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    mtmd_contexts: HashMap<String, Arc<MtmdContext>>,
}

impl MultiModelProvider {
    /// Create a new multi-model provider
    ///
    /// # Arguments
    /// * `registry` - The model registry containing loaded models
    /// * `default_model` - Default model name to use when none specified
    /// * `config` - Sampling configuration for generation
    pub fn new(registry: Arc<ModelRegistry>, default_model: &str, config: SamplingConfig) -> Self {
        Self {
            registry,
            default_model: default_model.to_string(),
            config,
            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            mtmd_contexts: HashMap::new(),
        }
    }

    /// Set the default model to use when none is specified
    pub fn set_default_model(&mut self, model: &str) {
        self.default_model = model.to_string();
    }

    /// Get the default model name
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Check if a model is available in the registry
    pub fn has_model(&self, name: &str) -> bool {
        self.registry.is_loaded(name)
    }

    /// Get list of available models
    pub fn available_models(&self) -> Vec<String> {
        self.registry.model_names()
    }
}

#[async_trait]
impl Provider for MultiModelProvider {
    async fn complete_with_options(
        &self,
        _messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        // For now, delegate to the default model
        // This is a placeholder - full implementation would:
        // 1. Determine which model to use (from request context or routing)
        // 2. Acquire context from registry
        // 3. Run inference
        // 4. Return result

        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        {
            // Ensure the default model is available
            if !self.registry.is_loaded(&self.default_model) {
                return Err(Error::Config(format!(
                    "Default model '{}' not found in registry",
                    self.default_model
                )));
            }

            // Placeholder implementation
            Err(Error::NotImplemented(
                "MultiModelProvider.complete_with_options not yet fully implemented".to_string(),
            ))
        }

        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            Err(Error::Config(
                "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
            ))
        }
    }

    async fn stream(
        &self,
        _messages: Vec<Message>,
    ) -> Result<Box<dyn tokio_stream::Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        {
            if !self.registry.is_loaded(&self.default_model) {
                return Err(Error::Config(format!(
                    "Default model '{}' not found in registry",
                    self.default_model
                )));
            }

            Err(Error::NotImplemented(
                "MultiModelProvider.stream not yet fully implemented".to_string(),
            ))
        }

        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            Err(Error::Config(
                "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
            ))
        }
    }

    fn name(&self) -> &str {
        &self.default_model
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete_with_tools(
        &self,
        _messages: Vec<Message>,
        _tools: Option<Value>,
        _max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        {
            if !self.registry.is_loaded(&self.default_model) {
                return Err(Error::Config(format!(
                    "Default model '{}' not found in registry",
                    self.default_model
                )));
            }

            Err(Error::NotImplemented(
                "MultiModelProvider.complete_with_tools not yet fully implemented".to_string(),
            ))
        }

        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            Err(Error::Config(
                "llama-cpp feature not enabled - rebuild with --features llama-cpp".to_string(),
            ))
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_multi_model_provider_creation() {
        let registry = Arc::new(ModelRegistry::new());
        let provider = MultiModelProvider::new(registry, "default", SamplingConfig::default());

        assert_eq!(provider.name(), "default");
        assert!(provider.supports_tools());
    }

    #[tokio::test]
    async fn test_multi_model_provider_set_default() {
        let registry = Arc::new(ModelRegistry::new());
        let mut provider = MultiModelProvider::new(registry, "model-a", SamplingConfig::default());

        assert_eq!(provider.default_model(), "model-a");

        provider.set_default_model("model-b");
        assert_eq!(provider.default_model(), "model-b");
        assert_eq!(provider.name(), "model-b");
    }

    #[tokio::test]
    async fn test_multi_model_provider_has_model() {
        let registry = Arc::new(ModelRegistry::new());
        let provider = MultiModelProvider::new(registry, "default", SamplingConfig::default());

        // Empty registry should have no models
        assert!(!provider.has_model("any-model"));
    }

    #[tokio::test]
    async fn test_multi_model_provider_available_models() {
        let registry = Arc::new(ModelRegistry::new());
        let provider = MultiModelProvider::new(registry, "default", SamplingConfig::default());

        // Empty registry should have no models
        let models = provider.available_models();
        assert!(models.is_empty());
    }

    /// Test thread safety
    #[test]
    fn test_multi_model_provider_thread_safety() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MultiModelProvider>();
    }
}
