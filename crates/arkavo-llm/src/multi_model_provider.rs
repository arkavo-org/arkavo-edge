//! Multi-Model Provider - Concurrent Inference Support
//!
//! Implements the Provider trait for multi-model inference, allowing
//! dynamic model selection per-request while maintaining the same
//! interface as single-model providers.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use crate::{
    Message, ModelRegistry, Provider, ProviderResponse, Result, SamplingConfig, StreamResponse,
};

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
use crate::{Error, Message, Provider, ProviderResponse, Result, StreamResponse};

/// Stub type for non-llama-cpp builds
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub struct ModelRegistry;

/// Stub type for non-llama-cpp builds
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
#[derive(Clone, Default)]
pub struct SamplingConfig;

/// Provider that can route requests to multiple loaded models
///
/// Uses a ModelRegistry to access different models and can select
/// which model to use based on request parameters or routing logic.
pub struct MultiModelProvider {
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    registry: Arc<ModelRegistry>,
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    _registry: Arc<ModelRegistry>,
    default_model: String,
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    config: SamplingConfig,
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    _config: SamplingConfig,
}

impl MultiModelProvider {
    /// Create a new multi-model provider
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn new(registry: Arc<ModelRegistry>, default_model: &str, config: SamplingConfig) -> Self {
        Self {
            registry,
            default_model: default_model.to_string(),
            config,
        }
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn new(
        _registry: Arc<ModelRegistry>,
        default_model: &str,
        _config: SamplingConfig,
    ) -> Self {
        Self {
            _registry: Arc::new(ModelRegistry),
            default_model: default_model.to_string(),
            _config: SamplingConfig,
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
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn has_model(&self, name: &str) -> bool {
        self.registry.is_loaded(name)
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn has_model(&self, _name: &str) -> bool {
        false
    }

    /// Get list of available models
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn available_models(&self) -> Vec<String> {
        self.registry.model_names()
    }

    /// Stub for non-llama-cpp builds
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    pub fn available_models(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Provider implementation for llama-cpp builds - delegates to LlamaCppProvider
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[async_trait]
impl Provider for MultiModelProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        // Create a LlamaCppProvider using the registry for the default model
        let provider = crate::LlamaCppProvider::new_with_registry(
            self.registry.clone(),
            self.default_model.clone(),
            self.config.clone(),
        )?;
        provider.complete_with_options(messages, max_tokens).await
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn tokio_stream::Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let provider = crate::LlamaCppProvider::new_with_registry(
            self.registry.clone(),
            self.default_model.clone(),
            self.config.clone(),
        )?;
        provider.stream(messages).await
    }

    fn name(&self) -> &str {
        &self.default_model
    }

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        let provider = crate::LlamaCppProvider::new_with_registry(
            self.registry.clone(),
            self.default_model.clone(),
            self.config.clone(),
        )?;
        provider
            .complete_with_tools(messages, tools, max_tokens)
            .await
    }
}

/// Stub Provider implementation for non-llama-cpp builds
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
#[async_trait]
impl Provider for MultiModelProvider {
    async fn complete_with_options(
        &self,
        _messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        Err(Error::Config(
            "MultiModelProvider requires llama-cpp feature".to_string(),
        ))
    }

    async fn stream(
        &self,
        _messages: Vec<Message>,
    ) -> Result<Box<dyn tokio_stream::Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        Err(Error::Config(
            "MultiModelProvider requires llama-cpp feature".to_string(),
        ))
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
        Err(Error::Config(
            "MultiModelProvider requires llama-cpp feature".to_string(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_multi_model_provider_creation() {
        let registry = Arc::new(ModelRegistry);
        let provider = MultiModelProvider::new(registry, "default", SamplingConfig::default());

        assert_eq!(provider.name(), "default");
        assert!(provider.supports_tools());
    }

    #[tokio::test]
    async fn test_multi_model_provider_set_default() {
        let registry = Arc::new(ModelRegistry);
        let mut provider = MultiModelProvider::new(registry, "model-a", SamplingConfig::default());

        assert_eq!(provider.default_model(), "model-a");

        provider.set_default_model("model-b");
        assert_eq!(provider.default_model(), "model-b");
        assert_eq!(provider.name(), "model-b");
    }

    #[tokio::test]
    async fn test_multi_model_provider_has_model() {
        let registry = Arc::new(ModelRegistry);
        let provider = MultiModelProvider::new(registry, "default", SamplingConfig::default());

        assert!(!provider.has_model("any-model"));
    }

    #[tokio::test]
    async fn test_multi_model_provider_available_models() {
        let registry = Arc::new(ModelRegistry);
        let provider = MultiModelProvider::new(registry, "default", SamplingConfig::default());

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
