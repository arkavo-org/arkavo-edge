//! Adapter to use arkavo-deepseek provider with arkavo-llm

use crate::tool_parser::ParsedToolCall;
use crate::{Error, Message, Provider, ProviderResponse, Result, Role, StreamResponse};
use arkavo_deepseek::{
    ChatMessage, DeepSeekConfig, DeepSeekProvider as InnerDeepSeekProvider, MessageContent, Tool,
    ToolFunction,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
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

    /// Create a V3.2-Speciale provider for planning tasks (no tools)
    ///
    /// Note: This endpoint expires on 2025-12-15. After that date, this
    /// constructor will return an error.
    ///
    /// # Panics
    ///
    /// This function will not panic as the date/time values are hardcoded
    /// valid values.
    pub fn v32_speciale() -> Result<Self> {
        // Safety guard: V3.2-Speciale endpoint expires on 2025-12-15
        let expiration = chrono::NaiveDate::from_ymd_opt(2025, 12, 15)
            .expect("valid date")
            .and_hms_opt(23, 59, 59)
            .expect("valid time");
        let expiration_utc = expiration.and_utc();

        if Utc::now() > expiration_utc {
            return Err(Error::Config(
                "DeepSeek V3.2-Speciale endpoint expired on 2025-12-15. \
                 Use standard DeepSeek models instead."
                    .into(),
            ));
        }

        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .map_err(|_| Error::Config("DEEPSEEK_API_KEY not set".into()))?;

        let config = DeepSeekConfig {
            api_key,
            base_url: "https://api.deepseek.com/v3.2_speciale_expires_on_20251215".to_string(),
            model: "deepseek-chat".to_string(),
            ..Default::default()
        };

        Self::new(config)
    }
}

/// Convert arkavo-llm messages to arkavo-deepseek ChatMessage
fn convert_messages_to_deepseek(messages: Vec<Message>) -> Vec<ChatMessage> {
    messages
        .into_iter()
        .map(|msg| ChatMessage {
            role: match msg.role {
                Role::System => arkavo_deepseek::Role::System,
                Role::User => arkavo_deepseek::Role::User,
                Role::Assistant => arkavo_deepseek::Role::Assistant,
            },
            content: MessageContent::Text {
                content: msg.content,
            },
            name: None,
            tool_calls: None,
            tool_call_id: None,
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
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        let deepseek_messages = convert_messages_to_deepseek(messages);

        let response = self
            .inner
            .complete_with_tools(deepseek_messages, None)
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
                    Error::Provider(format!("Authentication failed: {message}"))
                }
                arkavo_deepseek::DeepSeekError::ModelNotFound { model } => {
                    Error::Provider(format!("Model not found: {model}"))
                }
                _ => Error::Provider(e.to_string()),
            })?;

        response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| Error::Provider("No content in response".into()))
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        // Convert to provider Message format for stream
        let provider_messages: Vec<arkavo_deepseek::provider::Message> = messages
            .into_iter()
            .map(|msg| arkavo_deepseek::provider::Message {
                role: match msg.role {
                    Role::System => arkavo_deepseek::provider::MessageRole::System,
                    Role::User => arkavo_deepseek::provider::MessageRole::User,
                    Role::Assistant => arkavo_deepseek::provider::MessageRole::Assistant,
                },
                content: msg.content,
                images: msg.images,
            })
            .collect();

        // Use the arkavo-deepseek Provider trait
        use arkavo_deepseek::Provider as DeepSeekProviderTrait;

        let deepseek_stream = self
            .inner
            .stream(provider_messages)
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

    fn supports_tools(&self) -> bool {
        true
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        _max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        let deepseek_messages = convert_messages_to_deepseek(messages);

        let tool_defs = tools.and_then(|t| Self::convert_tools_to_deepseek(&t).ok());

        let response = self
            .inner
            .complete_with_tools(deepseek_messages, tool_defs)
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
                    Error::Provider(format!("Authentication failed: {message}"))
                }
                arkavo_deepseek::DeepSeekError::ModelNotFound { model } => {
                    Error::Provider(format!("Model not found: {model}"))
                }
                _ => Error::Provider(e.to_string()),
            })?;

        let first_choice = response
            .choices
            .first()
            .ok_or_else(|| Error::Provider("No choices in response".into()))?;

        let parsed_tool_calls = first_choice
            .message
            .tool_calls
            .as_ref()
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|tc| {
                        let args: Value = serde_json::from_str(&tc.function.arguments).ok()?;
                        Some(ParsedToolCall {
                            tool_name: tc.function.name.clone(),
                            arguments: args,
                            call_id: Some(tc.id.clone()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let finish_reason = first_choice.finish_reason.clone();

        Ok(ProviderResponse {
            content: first_choice.message.content.clone().unwrap_or_default(),
            tool_calls: parsed_tool_calls,
            finish_reason,
        })
    }
}

impl DeepSeekProvider {
    fn convert_tools_to_deepseek(tools_json: &Value) -> Result<Vec<Tool>> {
        let tools_array = tools_json
            .as_array()
            .ok_or_else(|| Error::Provider("Tools must be an array".into()))?;

        tools_array
            .iter()
            .map(|tool| {
                Ok(Tool {
                    tool_type: "function".to_string(),
                    function: ToolFunction {
                        name: tool
                            .get("name")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| Error::Provider("Tool missing name".into()))?
                            .to_string(),
                        description: tool
                            .get("description")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| Error::Provider("Tool missing description".into()))?
                            .to_string(),
                        parameters: tool
                            .get("input_schema")
                            .cloned()
                            .ok_or_else(|| Error::Provider("Tool missing input_schema".into()))?,
                    },
                    strict: None,
                })
            })
            .collect()
    }
}
