use crate::client::{KimiClient, KimiConfig};
use crate::error::KimiError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_stream::Stream;
use tracing::debug;

// Re-export types to avoid circular dependency with arkavo-llm
// These match the arkavo-llm types exactly

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamResponse {
    pub content: String,
    pub done: bool,
}

pub type LlmResult<T> = Result<T, LlmError>;

#[derive(Debug)]
pub enum LlmError {
    Config(String),
    Provider(String),
    Stream(String),
}

/// Kimi provider implementation for arkavo-llm
pub struct KimiProvider {
    client: KimiClient,
    temperature: Option<f32>,
    top_p: Option<f32>,
}

impl KimiProvider {
    /// Create a new Kimi provider
    pub fn new(config: KimiConfig) -> LlmResult<Self> {
        let client = KimiClient::new(config).map_err(|e| LlmError::Config(e.to_string()))?;

        Ok(Self {
            client,
            temperature: Some(0.7),
            top_p: None,
        })
    }

    /// Create from environment variables
    pub fn from_env() -> LlmResult<Self> {
        let config = KimiConfig::from_env().map_err(|e| LlmError::Config(e.to_string()))?;
        Self::new(config)
    }

    /// Set temperature for generation
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set top_p for generation
    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.top_p = Some(top_p);
        self
    }

    pub fn client(&self) -> &KimiClient {
        &self.client
    }
}

// This trait will be implemented when used through arkavo-llm
// to avoid circular dependency
#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, messages: Vec<Message>) -> LlmResult<String>;
    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> LlmResult<Box<dyn Stream<Item = LlmResult<StreamResponse>> + Send + Unpin>>;
    fn name(&self) -> &'static str;
}

#[async_trait]
impl Provider for KimiProvider {
    async fn complete(&self, messages: Vec<Message>) -> LlmResult<String> {
        debug!("Kimi provider completing with {} messages", messages.len());

        self.client
            .create_chat_completion(
                messages,
                None, // No tools for simple completion
                None, // No tool choice
                self.temperature,
                self.top_p,
            )
            .await
            .map_err(|e| match e {
                KimiError::AuthenticationFailed { message } => {
                    LlmError::Config(format!("Authentication failed: {message}"))
                }
                KimiError::RateLimitExceeded { message, .. } => LlmError::Provider(format!(
                    "Rate limit exceeded: {}",
                    message.unwrap_or_else(|| "Unknown".to_string())
                )),
                KimiError::ModelNotFound { model } => {
                    LlmError::Provider(format!("Model not found: {model}"))
                }
                _ => LlmError::Provider(e.to_string()),
            })
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> LlmResult<Box<dyn Stream<Item = LlmResult<StreamResponse>> + Send + Unpin>> {
        debug!("Kimi provider streaming with {} messages", messages.len());

        let stream = self
            .client
            .create_chat_completion_stream(
                messages,
                None, // No tools for simple streaming
                None, // No tool choice
                self.temperature,
                self.top_p,
            )
            .await
            .map_err(|e| match e {
                KimiError::AuthenticationFailed { message } => {
                    LlmError::Config(format!("Authentication failed: {message}"))
                }
                KimiError::RateLimitExceeded { message, .. } => LlmError::Provider(format!(
                    "Rate limit exceeded: {}",
                    message.unwrap_or_else(|| "Unknown".to_string())
                )),
                KimiError::ModelNotFound { model } => {
                    LlmError::Provider(format!("Model not found: {model}"))
                }
                _ => LlmError::Provider(e.to_string()),
            })?;

        // Convert KimiError to LlmError in the stream
        let mapped_stream = futures::StreamExt::map(stream, |result| {
            result.map_err(|e| LlmError::Stream(e.to_string()))
        });

        Ok(Box::new(Box::pin(mapped_stream)))
    }

    fn name(&self) -> &'static str {
        "kimi"
    }
}
