//! Adapter to use arkavo-deepseek provider with arkavo-llm

use crate::{Error, Message, Provider, Result, Role, StreamResponse};
use arkavo_deepseek::{DeepSeekConfig, DeepSeekProvider as InnerDeepSeekProvider};
use async_trait::async_trait;
use tokio_stream::Stream;

/// DeepSeek provider adapter for arkavo-llm
pub struct DeepSeekProvider {
    inner: InnerDeepSeekProvider,
}

impl DeepSeekProvider {
    /// Create a new DeepSeek provider
    pub fn new(config: DeepSeekConfig) -> Result<Self> {
        let inner = InnerDeepSeekProvider::new(config).map_err(|e| match e {
            arkavo_deepseek::DeepSeekError::ConfigError { message } => Error::Config(message),
            arkavo_deepseek::DeepSeekError::NetworkError { message } => Error::Provider(message),
            arkavo_deepseek::DeepSeekError::StreamError { message } => Error::Stream(message),
            _ => Error::Provider(e.to_string()),
        })?;

        Ok(Self { inner })
    }

    /// Create from environment variables
    pub fn from_env() -> Result<Self> {
        let inner = InnerDeepSeekProvider::from_env().map_err(|e| match e {
            arkavo_deepseek::DeepSeekError::ConfigError { message } => Error::Config(message),
            arkavo_deepseek::DeepSeekError::NetworkError { message } => Error::Provider(message),
            arkavo_deepseek::DeepSeekError::StreamError { message } => Error::Stream(message),
            _ => Error::Provider(e.to_string()),
        })?;

        Ok(Self { inner })
    }

    /// Set temperature for generation
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.inner = self.inner.with_temperature(temperature);
        self
    }

    /// Set top_p for generation
    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.inner = self.inner.with_top_p(top_p);
        self
    }

    /// Enable strict mode
    pub fn with_strict_mode(mut self, enabled: bool) -> Self {
        self.inner = self.inner.with_strict_mode(enabled);
        self
    }
}

/// Convert arkavo-llm messages to arkavo-deepseek messages
fn convert_messages_to_deepseek(messages: Vec<Message>) -> Vec<arkavo_deepseek::Message> {
    messages
        .into_iter()
        .map(|msg| arkavo_deepseek::Message {
            role: match msg.role {
                Role::System => arkavo_deepseek::MessageRole::System,
                Role::User => arkavo_deepseek::MessageRole::User,
                Role::Assistant => arkavo_deepseek::MessageRole::Assistant,
            },
            content: msg.content,
            images: msg.images,
        })
        .collect()
}

/// Convert arkavo-deepseek stream response to arkavo-llm stream response
fn convert_stream_response(resp: arkavo_deepseek::StreamResponse) -> StreamResponse {
    StreamResponse {
        content: resp.content.unwrap_or_default(),
        done: resp.done,
    }
}

#[async_trait]
impl Provider for DeepSeekProvider {
    async fn complete_with_options(&self, messages: Vec<Message>, _max_tokens: Option<usize>) -> Result<String> {
        let deepseek_messages = convert_messages_to_deepseek(messages);

        // Use the arkavo-deepseek Provider trait
        use arkavo_deepseek::Provider as DeepSeekProviderTrait;

        self.inner
            .complete(deepseek_messages)
            .await
            .map_err(|e| match e {
                arkavo_deepseek::DeepSeekError::ConfigError { message } => Error::Config(message),
                arkavo_deepseek::DeepSeekError::NetworkError { message } => {
                    Error::Provider(message)
                }
                arkavo_deepseek::DeepSeekError::StreamError { message } => Error::Stream(message),
                arkavo_deepseek::DeepSeekError::RateLimited { message, .. } => {
                    Error::Provider(message.unwrap_or_else(|| "Rate limited".to_string()))
                }
                arkavo_deepseek::DeepSeekError::AuthenticationFailed { message } => {
                    Error::Provider(format!("Authentication failed: {}", message))
                }
                arkavo_deepseek::DeepSeekError::ModelNotFound { model } => {
                    Error::Provider(format!("Model not found: {}", model))
                }
                _ => Error::Provider(e.to_string()),
            })
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let deepseek_messages = convert_messages_to_deepseek(messages);

        // Use the arkavo-deepseek Provider trait
        use arkavo_deepseek::Provider as DeepSeekProviderTrait;

        let deepseek_stream = self
            .inner
            .stream(deepseek_messages)
            .await
            .map_err(|e| match e {
                arkavo_deepseek::DeepSeekError::ConfigError { message } => Error::Config(message),
                arkavo_deepseek::DeepSeekError::NetworkError { message } => {
                    Error::Provider(message)
                }
                arkavo_deepseek::DeepSeekError::StreamError { message } => Error::Stream(message),
                _ => Error::Provider(e.to_string()),
            })?;

        // Convert the stream
        let mapped_stream = futures::StreamExt::map(deepseek_stream, |result| {
            result.map(convert_stream_response).map_err(|e| match e {
                arkavo_deepseek::DeepSeekError::ConfigError { message } => Error::Config(message),
                arkavo_deepseek::DeepSeekError::NetworkError { message } => {
                    Error::Provider(message)
                }
                arkavo_deepseek::DeepSeekError::StreamError { message } => Error::Stream(message),
                _ => Error::Provider(e.to_string()),
            })
        });

        Ok(Box::new(Box::pin(mapped_stream)))
    }

    fn name(&self) -> &'static str {
        // Use the arkavo-deepseek Provider trait
        use arkavo_deepseek::Provider as DeepSeekProviderTrait;
        self.inner.name()
    }
}
