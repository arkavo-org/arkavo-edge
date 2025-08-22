#[cfg(feature = "llm-remote")]
pub mod anthropic;
#[cfg(feature = "llm-remote")]
pub mod factory;
#[cfg(feature = "llm-remote")]
pub mod health;
#[cfg(feature = "llm-remote")]
pub mod openai;

#[cfg(feature = "llm-remote")]
pub use anthropic::{AnthropicConfig, AnthropicProvider};
#[cfg(feature = "llm-remote")]
pub use factory::{
    AnthropicProviderFactory, OllamaProviderFactory, OpenAIProviderFactory, ProviderConfig,
    ProviderFactory, ProviderFactoryRegistry, ProviderType,
};
#[cfg(feature = "deepseek")]
pub use factory::DeepSeekProviderFactory;
#[cfg(feature = "llm-remote")]
pub use health::{
    CircuitBreaker, HealthCheckResult, HealthStatus, ProviderHealthMonitor, ProviderMetrics,
};
#[cfg(feature = "llm-remote")]
pub use openai::{OpenAIConfig, OpenAIProvider};
