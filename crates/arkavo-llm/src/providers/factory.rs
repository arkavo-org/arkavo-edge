use super::anthropic::{AnthropicConfig, AnthropicProvider};
use super::openai::{OpenAIConfig, OpenAIProvider};
use crate::ollama::OllamaClient;
use crate::provider::Provider;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Types of LLM providers supported
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    #[cfg(feature = "llm-remote")]
    Ollama,
    #[cfg(feature = "llm-remote")]
    OpenAI,
    #[cfg(feature = "llm-remote")]
    Anthropic,
    #[cfg(feature = "llm-remote")]
    Gemini,
    #[cfg(feature = "deepseek")]
    DeepSeek,
    #[cfg(feature = "llama-cpp")]
    Local,
    Custom(String),
}

impl ProviderType {
    pub fn from_name(name: &str) -> Self {
        let name_lower = name.to_lowercase();

        #[cfg(feature = "llm-remote")]
        {
            if name_lower.contains("ollama") {
                return ProviderType::Ollama;
            }
            if name_lower.contains("openai") {
                return ProviderType::OpenAI;
            }
            if name_lower.contains("anthropic") {
                return ProviderType::Anthropic;
            }
            if name_lower.contains("gemini") {
                return ProviderType::Gemini;
            }
        }

        #[cfg(feature = "deepseek")]
        {
            if name_lower.contains("deepseek") {
                return ProviderType::DeepSeek;
            }
        }

        #[cfg(feature = "llama-cpp")]
        {
            if name_lower.contains("local") {
                return ProviderType::Local;
            }
        }

        ProviderType::Custom(name.to_string())
    }
}

/// Configuration for creating a provider instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider_type: ProviderType,
    pub base_url: String,
    pub auth_ref: Option<String>,
    pub default_model: Option<String>,
    pub timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
    /// Initial retry delay in milliseconds (default: 1000)
    pub initial_retry_delay_ms: Option<u64>,
    /// Backoff factor for exponential retry (default: 2.0)
    pub backoff_factor: Option<f64>,
    /// Maximum retry delay in milliseconds (default: 30000)
    pub max_retry_delay_ms: Option<u64>,
    /// Jitter factor 0.0-1.0 (default: 0.1)
    pub jitter_factor: Option<f64>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Factory trait for creating provider instances
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    /// Create a new provider instance from configuration
    async fn create_provider(&self, config: &ProviderConfig) -> Result<Box<dyn Provider>>;

    /// Get the provider type this factory supports
    fn provider_type(&self) -> ProviderType;

    /// Validate configuration before creating provider
    async fn validate_config(&self, config: &ProviderConfig) -> Result<()>;
}

/// Factory for creating Ollama provider instances
#[cfg(feature = "llm-remote")]
pub struct OllamaProviderFactory;

#[cfg(feature = "llm-remote")]
#[async_trait]
impl ProviderFactory for OllamaProviderFactory {
    async fn create_provider(&self, config: &ProviderConfig) -> Result<Box<dyn Provider>> {
        // Validate configuration first
        self.validate_config(config).await?;

        // Create Ollama client
        let client = OllamaClient::new(Some(config.base_url.clone()), config.default_model.clone());

        Ok(Box::new(client))
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Ollama
    }

    async fn validate_config(&self, config: &ProviderConfig) -> Result<()> {
        if config.base_url.is_empty() {
            return Err(anyhow::anyhow!("Base URL is required for Ollama provider"));
        }

        // Validate URL format
        let url = url::Url::parse(&config.base_url)
            .map_err(|e| anyhow::anyhow!("Invalid base URL: {e}"))?;

        // Ensure it's HTTP or HTTPS
        if !["http", "https"].contains(&url.scheme()) {
            return Err(anyhow::anyhow!("URL must use HTTP or HTTPS scheme"));
        }

        Ok(())
    }
}

/// Registry of provider factories
pub struct ProviderFactoryRegistry {
    factories: HashMap<ProviderType, Arc<dyn ProviderFactory>>,
}

impl ProviderFactoryRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            factories: HashMap::new(),
        };

        // Register default factories
        #[cfg(feature = "llm-remote")]
        {
            registry.register(Arc::new(OllamaProviderFactory));
            registry.register(Arc::new(OpenAIProviderFactory));
            registry.register(Arc::new(AnthropicProviderFactory));
        }
        #[cfg(feature = "deepseek")]
        registry.register(Arc::new(DeepSeekProviderFactory));
        #[cfg(feature = "llama-cpp")]
        registry.register(Arc::new(LocalProviderFactory));

        registry
    }

    /// Register a new provider factory
    pub fn register(&mut self, factory: Arc<dyn ProviderFactory>) {
        self.factories.insert(factory.provider_type(), factory);
    }

    /// Get a factory for a provider type
    pub fn get_factory(&self, provider_type: &ProviderType) -> Option<Arc<dyn ProviderFactory>> {
        self.factories.get(provider_type).cloned()
    }

    /// Create a provider instance from configuration
    pub async fn create_provider(&self, config: &ProviderConfig) -> Result<Box<dyn Provider>> {
        let factory = self.get_factory(&config.provider_type).ok_or_else(|| {
            anyhow::anyhow!(
                "No factory registered for provider type: {:?}",
                config.provider_type
            )
        })?;

        factory.create_provider(config).await
    }

    /// Get all registered provider types
    pub fn registered_types(&self) -> Vec<ProviderType> {
        self.factories.keys().cloned().collect()
    }
}

impl Default for ProviderFactoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Factory for creating OpenAI provider instances
#[cfg(feature = "llm-remote")]
pub struct OpenAIProviderFactory;

#[cfg(feature = "llm-remote")]
#[async_trait]
impl ProviderFactory for OpenAIProviderFactory {
    async fn create_provider(&self, config: &ProviderConfig) -> Result<Box<dyn Provider>> {
        if config.default_model.as_deref() == Some("gpt-6-astra") {
            let api_key = config
                .auth_ref
                .as_ref()
                .map(std::env::var)
                .transpose()
                .map_err(|_| anyhow::anyhow!("OpenAI credential is missing"))?;
            let mut responses = super::OpenAIResponsesConfig {
                api_key,
                ..Default::default()
            };
            if !config.base_url.is_empty() {
                responses.base_url.clone_from(&config.base_url);
            }
            if let Some(effort) = config
                .metadata
                .as_ref()
                .and_then(|m| m.get("reasoning_effort"))
            {
                responses.reasoning_effort = serde_json::from_value(effort.clone())?;
            }
            return Ok(Box::new(super::OpenAIResponsesProvider::new(responses)?));
        }
        // Get API key from auth manager if auth_ref is provided
        let api_key = if let Some(ref auth_ref) = config.auth_ref {
            // See #204: Re-enable AuthManager when available in arkavo-llm
            std::env::var(auth_ref).map_err(|_| {
                anyhow::anyhow!(
                    "Credential '{}' not found in environment",
                    auth_ref.chars().take(8).collect::<String>() + "..."
                )
            })?
        } else {
            return Err(anyhow::anyhow!(
                "API key required for OpenAI provider. Set auth_ref in the node configuration or provide OPENAI_API_KEY environment variable"
            ));
        };

        // Check if this is an Azure endpoint
        let is_azure = config.base_url.contains("azure.com");

        let openai_config = OpenAIConfig {
            api_key,
            base_url: config.base_url.clone(),
            model: config
                .default_model
                .clone()
                .unwrap_or_else(|| "gpt-4".to_string()),
            organization_id: config
                .metadata
                .as_ref()
                .and_then(|m| m.get("organization_id"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            api_version: config
                .metadata
                .as_ref()
                .and_then(|m| m.get("api_version"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            is_azure,
        };

        let provider = OpenAIProvider::new(openai_config)?;
        Ok(Box::new(provider))
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::OpenAI
    }

    async fn validate_config(&self, config: &ProviderConfig) -> Result<()> {
        if config.default_model.as_deref() == Some("gpt-6-astra") {
            let responses = super::OpenAIResponsesConfig {
                base_url: if config.base_url.is_empty() {
                    super::OpenAIResponsesConfig::default().base_url
                } else {
                    config.base_url.clone()
                },
                ..Default::default()
            };
            responses.validate()?;
            return Ok(());
        }
        if config.auth_ref.is_none() {
            return Err(anyhow::anyhow!(
                "API key required for OpenAI provider. Set auth_ref in the node configuration"
            ));
        }

        // Validate URL format
        let url = url::Url::parse(&config.base_url)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(anyhow::anyhow!("Invalid URL scheme for OpenAI provider"));
        }

        Ok(())
    }
}

/// Factory for creating Anthropic provider instances
#[cfg(feature = "llm-remote")]
pub struct AnthropicProviderFactory;

#[cfg(feature = "llm-remote")]
#[async_trait]
impl ProviderFactory for AnthropicProviderFactory {
    async fn create_provider(&self, config: &ProviderConfig) -> Result<Box<dyn Provider>> {
        // Get API key from auth manager if auth_ref is provided
        let api_key = if let Some(ref auth_ref) = config.auth_ref {
            // See #204: Re-enable AuthManager when available in arkavo-llm
            std::env::var(auth_ref).map_err(|_| {
                anyhow::anyhow!(
                    "Credential '{}' not found in environment",
                    auth_ref.chars().take(8).collect::<String>() + "..."
                )
            })?
        } else {
            return Err(anyhow::anyhow!(
                "API key required for Anthropic provider. Set auth_ref in the node configuration or provide ANTHROPIC_API_KEY environment variable"
            ));
        };

        let anthropic_config = AnthropicConfig {
            api_key,
            base_url: config.base_url.clone(),
            model: config
                .default_model
                .clone()
                .unwrap_or_else(|| "claude-3-opus-20240229".to_string()),
            api_version: config
                .metadata
                .as_ref()
                .and_then(|m| m.get("api_version"))
                .and_then(|v| v.as_str())
                .map_or_else(
                    || "2023-06-01".to_string(),
                    std::string::ToString::to_string,
                ),
        };

        let provider = AnthropicProvider::new(anthropic_config)?;
        Ok(Box::new(provider))
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Anthropic
    }

    async fn validate_config(&self, config: &ProviderConfig) -> Result<()> {
        if config.auth_ref.is_none() {
            return Err(anyhow::anyhow!(
                "API key required for Anthropic provider. Set auth_ref in the node configuration"
            ));
        }

        // Validate URL format
        let url = url::Url::parse(&config.base_url)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(anyhow::anyhow!("Invalid URL scheme for Anthropic provider"));
        }

        Ok(())
    }
}

/// Factory for creating DeepSeek provider instances
#[cfg(feature = "deepseek")]
pub struct DeepSeekProviderFactory;

#[cfg(feature = "deepseek")]
#[async_trait]
impl ProviderFactory for DeepSeekProviderFactory {
    async fn create_provider(&self, config: &ProviderConfig) -> Result<Box<dyn Provider>> {
        // Get API key from auth manager if auth_ref is provided
        let api_key = if let Some(ref auth_ref) = config.auth_ref {
            // See #204: Re-enable AuthManager when available in arkavo-llm
            std::env::var(auth_ref).map_err(|_| {
                anyhow::anyhow!(
                    "Credential '{}' not found in environment",
                    auth_ref.chars().take(8).collect::<String>() + "..."
                )
            })?
        } else {
            return Err(anyhow::anyhow!(
                "API key required for DeepSeek provider. Set auth_ref in the node configuration or provide DEEPSEEK_API_KEY environment variable"
            ));
        };

        // Use feature flag to import DeepSeek types
        #[cfg(feature = "deepseek")]
        use crate::DeepSeekProvider;
        #[cfg(feature = "deepseek")]
        use arkavo_deepseek::DeepSeekConfig;

        let use_strict_mode = config
            .metadata
            .as_ref()
            .and_then(|m| m.get("strict_mode"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let anthropic_compat = config
            .metadata
            .as_ref()
            .and_then(|m| m.get("anthropic_compat"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let deepseek_config = DeepSeekConfig {
            api_key,
            base_url: if use_strict_mode {
                "https://api.deepseek.com/beta".to_string()
            } else if anthropic_compat {
                "https://api.deepseek.com/anthropic".to_string()
            } else {
                config.base_url.clone()
            },
            model: config
                .default_model
                .clone()
                .unwrap_or_else(|| "deepseek-chat".to_string()),
            use_strict_mode,
            anthropic_compat,
            thinking_mode: config
                .metadata
                .as_ref()
                .and_then(|m| m.get("thinking_mode"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            max_tokens: config
                .metadata
                .as_ref()
                .and_then(|m| m.get("max_tokens"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            temperature: config
                .metadata
                .as_ref()
                .and_then(|m| m.get("temperature"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            top_p: config
                .metadata
                .as_ref()
                .and_then(|m| m.get("top_p"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            timeout: std::time::Duration::from_secs(config.timeout_secs.unwrap_or(60)),
            max_retries: config.max_retries.unwrap_or(3),
        };

        let provider = DeepSeekProvider::new(deepseek_config)?;
        Ok(Box::new(provider))
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::DeepSeek
    }

    async fn validate_config(&self, config: &ProviderConfig) -> Result<()> {
        if config.auth_ref.is_none() {
            return Err(anyhow::anyhow!(
                "API key required for DeepSeek provider. Set auth_ref in the node configuration"
            ));
        }

        // Validate URL format
        let url = url::Url::parse(&config.base_url)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(anyhow::anyhow!("Invalid URL scheme for DeepSeek provider"));
        }

        Ok(())
    }
}

/// Factory for creating Local provider instances
#[cfg(feature = "llama-cpp")]
pub struct LocalProviderFactory;

#[cfg(feature = "llama-cpp")]
#[async_trait]
impl ProviderFactory for LocalProviderFactory {
    async fn create_provider(&self, config: &ProviderConfig) -> Result<Box<dyn Provider>> {
        // Validate configuration first
        self.validate_config(config).await?;

        // Extract model name and path from config
        let model_name = config
            .default_model
            .clone()
            .unwrap_or_else(|| "local-model".to_string());

        let model_path = config
            .metadata
            .as_ref()
            .and_then(|m| m.get("model_path"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);

        // Create llama-cpp provider
        let provider = crate::LlamaCppProvider::new(
            model_name,
            model_path
                .ok_or_else(|| anyhow::anyhow!("model_path is required for local provider"))?,
        )?;

        Ok(Box::new(provider))
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Local
    }

    async fn validate_config(&self, config: &ProviderConfig) -> Result<()> {
        // Local provider doesn't require auth_ref
        // but might require a model path in metadata

        // If base_url is provided, it should be a local:// URL
        if !config.base_url.is_empty() && !config.base_url.starts_with("local://") {
            return Err(anyhow::anyhow!(
                "Local provider requires 'local://' URL scheme, got: {}",
                config.base_url
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_type_from_name() {
        #[cfg(feature = "llm-remote")]
        #[cfg(feature = "llm-remote")]
        {
            assert_eq!(
                ProviderType::from_name("local-ollama"),
                ProviderType::Ollama
            );
            assert_eq!(
                ProviderType::from_name("ollama-remote"),
                ProviderType::Ollama
            );
            assert_eq!(ProviderType::from_name("openai"), ProviderType::OpenAI);
        }
        #[cfg(feature = "llama-cpp")]
        {
            assert_eq!(ProviderType::from_name("local"), ProviderType::Local);
            assert_eq!(ProviderType::from_name("local-gemma"), ProviderType::Local);
        }
        assert_eq!(
            ProviderType::from_name("custom-llm"),
            ProviderType::Custom("custom-llm".to_string())
        );
    }

    #[cfg(feature = "llm-remote")]
    #[tokio::test]
    async fn test_ollama_factory_validation() {
        let factory = OllamaProviderFactory;

        // Test invalid URL
        let config = ProviderConfig {
            provider_type: ProviderType::Ollama,
            base_url: String::new(),
            auth_ref: None,
            default_model: None,
            timeout_secs: None,
            max_retries: None,
            initial_retry_delay_ms: None,
            backoff_factor: None,
            max_retry_delay_ms: None,
            jitter_factor: None,
            metadata: None,
        };

        assert!(factory.validate_config(&config).await.is_err());

        // Test valid URL
        let config = ProviderConfig {
            provider_type: ProviderType::Ollama,
            base_url: "http://localhost:11434".to_string(),
            auth_ref: None,
            default_model: None,
            timeout_secs: None,
            max_retries: None,
            initial_retry_delay_ms: None,
            backoff_factor: None,
            max_retry_delay_ms: None,
            jitter_factor: None,
            metadata: None,
        };

        assert!(factory.validate_config(&config).await.is_ok());
    }

    #[test]
    fn test_registry() {
        let registry = ProviderFactoryRegistry::new();

        // Check default registered types
        let types = registry.registered_types();
        #[cfg(feature = "llm-remote")]
        {
            assert!(types.contains(&ProviderType::Ollama));
            assert!(types.contains(&ProviderType::OpenAI));
            assert!(types.contains(&ProviderType::Anthropic));
        }
        #[cfg(feature = "llama-cpp")]
        assert!(types.contains(&ProviderType::Local));

        // Get factory
        #[cfg(feature = "llm-remote")]
        {
            assert!(registry.get_factory(&ProviderType::Ollama).is_some());
            assert!(registry.get_factory(&ProviderType::OpenAI).is_some());
            assert!(registry.get_factory(&ProviderType::Anthropic).is_some());
        }
        #[cfg(feature = "llama-cpp")]
        assert!(registry.get_factory(&ProviderType::Local).is_some());
    }
}
