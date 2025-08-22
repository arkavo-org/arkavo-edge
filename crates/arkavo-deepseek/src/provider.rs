use crate::client::{DeepSeekClient, DeepSeekConfig};
use crate::error::{DeepSeekError, Result as DeepSeekResult};
use crate::stream::StreamResponse;
use crate::types::{ChatMessage, MessageContent, Role};
use async_trait::async_trait;
use futures::Stream;

/// Provider error type alias for external compatibility
pub type LlmError = DeepSeekError;
pub type LlmResult<T> = DeepSeekResult<T>;

/// Provider trait for DeepSeek - matching arkavo-kimi pattern
#[async_trait]
pub trait Provider: Send + Sync {
    /// Complete a conversation
    async fn complete(&self, messages: Vec<Message>) -> LlmResult<String>;

    /// Stream a conversation
    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> LlmResult<Box<dyn Stream<Item = LlmResult<StreamResponse>> + Send + Unpin>>;

    /// Get provider name
    fn name(&self) -> &'static str;
}

/// Message structure matching arkavo-kimi pattern
#[derive(Debug, Clone)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub images: Option<Vec<String>>,
}

/// Message role matching arkavo-kimi pattern
#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

/// DeepSeek provider implementation
pub struct DeepSeekProvider {
    client: DeepSeekClient,
    config: DeepSeekConfig,
}

impl DeepSeekProvider {
    /// Create a new DeepSeek provider
    pub fn new(config: DeepSeekConfig) -> LlmResult<Self> {
        let client = DeepSeekClient::new(config.clone())?;
        Ok(Self { client, config })
    }

    /// Create from environment variables
    pub fn from_env() -> LlmResult<Self> {
        let client = DeepSeekClient::from_env()?;
        let config = DeepSeekConfig {
            api_key: std::env::var("DEEPSEEK_API_KEY").unwrap_or_default(),
            base_url: std::env::var("DEEPSEEK_BASE_URL")
                .unwrap_or_else(|_| "https://api.deepseek.com".to_string()),
            model: std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string()),
            ..Default::default()
        };
        Ok(Self { client, config })
    }

    /// Set temperature for generation
    ///
    /// # Panics
    ///
    /// Panics if the client cannot be recreated with the new configuration
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.config.temperature = Some(temperature);
        self.client = DeepSeekClient::new(self.config.clone())
            .expect("Failed to recreate client with new temperature");
        self
    }

    /// Set top_p for generation
    ///
    /// # Panics
    ///
    /// Panics if the client cannot be recreated with the new configuration
    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.config.top_p = Some(top_p);
        self.client = DeepSeekClient::new(self.config.clone())
            .expect("Failed to recreate client with new top_p");
        self
    }

    /// Enable strict mode
    pub fn with_strict_mode(mut self, enabled: bool) -> Self {
        self.config.use_strict_mode = enabled;
        self.client = self.client.with_strict_mode(enabled);
        self
    }

    /// Convert messages to DeepSeek format
    fn convert_messages(&self, messages: Vec<Message>) -> Vec<ChatMessage> {
        messages
            .into_iter()
            .map(|msg| {
                let role = match msg.role {
                    MessageRole::System => Role::System,
                    MessageRole::User => Role::User,
                    MessageRole::Assistant => Role::Assistant,
                };

                // Note: DeepSeek doesn't support images in standard API
                if msg.images.is_some() {
                    tracing::warn!("Images are not supported by DeepSeek and will be ignored");
                }

                ChatMessage {
                    role,
                    content: MessageContent::Text {
                        content: msg.content,
                    },
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                }
            })
            .collect()
    }
}

#[async_trait]
impl Provider for DeepSeekProvider {
    async fn complete(&self, messages: Vec<Message>) -> LlmResult<String> {
        let deepseek_messages = self.convert_messages(messages);

        let response = self
            .client
            .complete(deepseek_messages, None, None, None)
            .await?;

        // Extract content from first choice
        if let Some(choice) = response.choices.first() {
            Ok(choice.message.content.clone().unwrap_or_default())
        } else {
            Err(DeepSeekError::InternalError {
                message: "No choices in response".to_string(),
            })
        }
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> LlmResult<Box<dyn Stream<Item = LlmResult<StreamResponse>> + Send + Unpin>> {
        let deepseek_messages = self.convert_messages(messages);

        let stream = self
            .client
            .stream(deepseek_messages, None, None, None)
            .await?;

        Ok(Box::new(stream)
            as Box<
                dyn Stream<Item = LlmResult<StreamResponse>> + Send + Unpin,
            >)
    }

    fn name(&self) -> &'static str {
        "deepseek"
    }
}

/// Configuration for DeepSeek provider
#[derive(Debug, Clone)]
pub struct KimiConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub use_strict_mode: bool,
}

impl From<KimiConfig> for DeepSeekConfig {
    fn from(config: KimiConfig) -> Self {
        DeepSeekConfig {
            api_key: config.api_key,
            base_url: config.base_url,
            model: config.model,
            temperature: config.temperature,
            top_p: config.top_p,
            max_tokens: config.max_tokens,
            use_strict_mode: config.use_strict_mode,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_conversion() {
        let messages = vec![
            Message {
                role: MessageRole::System,
                content: "You are a helpful assistant".to_string(),
                images: None,
            },
            Message {
                role: MessageRole::User,
                content: "Hello".to_string(),
                images: None,
            },
            Message {
                role: MessageRole::Assistant,
                content: "Hi there!".to_string(),
                images: None,
            },
        ];

        let config = DeepSeekConfig::default();
        let provider = DeepSeekProvider::new(config).unwrap();
        let converted = provider.convert_messages(messages);

        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0].role, Role::System);
        assert_eq!(converted[1].role, Role::User);
        assert_eq!(converted[2].role, Role::Assistant);
    }
}
