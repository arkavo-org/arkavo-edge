use anyhow::Result;
use arkavo_llm::ollama::OllamaClient;
use arkavo_llm::provider::Provider;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Types of LLM providers supported
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Ollama,
    OpenAI,
    Anthropic,
    Gemini,
    Custom(String),
}

impl ProviderType {
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            n if n.contains("ollama") => ProviderType::Ollama,
            n if n.contains("openai") => ProviderType::OpenAI,
            n if n.contains("anthropic") => ProviderType::Anthropic,
            n if n.contains("gemini") => ProviderType::Gemini,
            _ => ProviderType::Custom(name.to_string()),
        }
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
pub struct OllamaProviderFactory;

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
            .map_err(|e| anyhow::anyhow!("Invalid base URL: {}", e))?;

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
        registry.register(Arc::new(OllamaProviderFactory));

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

/// Placeholder factory for OpenAI (to be implemented)
pub struct OpenAIProviderFactory;

#[async_trait]
impl ProviderFactory for OpenAIProviderFactory {
    async fn create_provider(&self, _config: &ProviderConfig) -> Result<Box<dyn Provider>> {
        Err(anyhow::anyhow!("OpenAI provider not yet implemented"))
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::OpenAI
    }

    async fn validate_config(&self, config: &ProviderConfig) -> Result<()> {
        if config.auth_ref.is_none() {
            return Err(anyhow::anyhow!("API key required for OpenAI provider"));
        }
        Ok(())
    }
}

/// Placeholder factory for Anthropic (to be implemented)
pub struct AnthropicProviderFactory;

#[async_trait]
impl ProviderFactory for AnthropicProviderFactory {
    async fn create_provider(&self, _config: &ProviderConfig) -> Result<Box<dyn Provider>> {
        Err(anyhow::anyhow!("Anthropic provider not yet implemented"))
    }

    fn provider_type(&self) -> ProviderType {
        ProviderType::Anthropic
    }

    async fn validate_config(&self, config: &ProviderConfig) -> Result<()> {
        if config.auth_ref.is_none() {
            return Err(anyhow::anyhow!("API key required for Anthropic provider"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_type_from_name() {
        assert_eq!(
            ProviderType::from_name("local-ollama"),
            ProviderType::Ollama
        );
        assert_eq!(
            ProviderType::from_name("ollama-remote"),
            ProviderType::Ollama
        );
        assert_eq!(ProviderType::from_name("openai"), ProviderType::OpenAI);
        assert_eq!(
            ProviderType::from_name("custom-llm"),
            ProviderType::Custom("custom-llm".to_string())
        );
    }

    #[tokio::test]
    async fn test_ollama_factory_validation() {
        let factory = OllamaProviderFactory;

        // Test invalid URL
        let config = ProviderConfig {
            provider_type: ProviderType::Ollama,
            base_url: "".to_string(),
            auth_ref: None,
            default_model: None,
            timeout_secs: None,
            max_retries: None,
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
            metadata: None,
        };

        assert!(factory.validate_config(&config).await.is_ok());
    }

    #[test]
    fn test_registry() {
        let registry = ProviderFactoryRegistry::new();

        // Check default registered types
        let types = registry.registered_types();
        assert!(types.contains(&ProviderType::Ollama));

        // Get factory
        assert!(registry.get_factory(&ProviderType::Ollama).is_some());
        assert!(registry.get_factory(&ProviderType::OpenAI).is_none());
    }
}
