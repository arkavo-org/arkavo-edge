//! Adapter to use arkavo-kimi provider with arkavo-llm

use crate::{Error, LlmConfig, Message, Provider, Result, Role, StreamResponse};
use arkavo_kimi::{KimiConfig, KimiProvider as InnerKimiProvider};
use async_trait::async_trait;
use tokio_stream::Stream;

/// Kimi provider adapter for arkavo-llm
pub struct KimiProvider {
    inner: InnerKimiProvider,
}

impl KimiProvider {
    /// Create a new Kimi provider
    pub fn new(config: KimiConfig) -> Result<Self> {
        let inner = InnerKimiProvider::new(config).map_err(|e| match e {
            arkavo_kimi::provider::LlmError::Config(msg) => Error::Config(msg),
            arkavo_kimi::provider::LlmError::Provider(msg) => Error::Provider(msg),
            arkavo_kimi::provider::LlmError::Stream(msg) => Error::Stream(msg),
        })?;

        Ok(Self { inner })
    }

    /// Create from environment variables
    pub fn from_env() -> Result<Self> {
        let inner = InnerKimiProvider::from_env().map_err(|e| match e {
            arkavo_kimi::provider::LlmError::Config(msg) => Error::Config(msg),
            arkavo_kimi::provider::LlmError::Provider(msg) => Error::Provider(msg),
            arkavo_kimi::provider::LlmError::Stream(msg) => Error::Stream(msg),
        })?;

        Ok(Self { inner })
    }

    /// Create from LlmConfig
    pub fn from_config(config: &LlmConfig) -> Result<Self> {
        let api_key = config
            .get_api_key("MOONSHOT_API_KEY")
            .ok_or_else(|| Error::Config("MOONSHOT_API_KEY not provided in config".to_string()))?;

        let mut kimi_config = KimiConfig {
            api_key: api_key.clone(),
            ..Default::default()
        };

        if let Some(base_url) = &config.base_url {
            kimi_config.base_url.clone_from(base_url);
        }

        if let Some(model) = &config.model {
            kimi_config.model = match model.as_str() {
                "moonshot-v1-8k" => arkavo_kimi::Model::MoonshotV1_8k,
                "moonshot-v1-32k" => arkavo_kimi::Model::MoonshotV1_32k,
                "moonshot-v1-128k" => arkavo_kimi::Model::MoonshotV1_128k,
                // K2.5 series models
                "kimi-k2.5" => arkavo_kimi::Model::KimiK2_5,
                "kimi-k2-0905-preview" => arkavo_kimi::Model::KimiK20905Preview,
                "kimi-k2-turbo-preview" => arkavo_kimi::Model::KimiK2TurboPreview,
                "kimi-k2-thinking" => arkavo_kimi::Model::KimiK2Thinking,
                "kimi-k2-thinking-turbo" => arkavo_kimi::Model::KimiK2ThinkingTurbo,
                _ => arkavo_kimi::Model::KimiK2_5, // Default to K2.5
            };
        }

        Self::new(kimi_config)
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
}

/// Convert arkavo-llm messages to arkavo-kimi messages.
///
/// Tool results become user turns: Moonshot continues a trailing assistant
/// message rather than answering it, so replaying a result under that role
/// makes the model finish its own tool output. The wire crate exposes no tool
/// role, and this adapter forwards no assistant `tool_calls` that a tool role
/// would have to pair with, so user text is the only well-formed carrier.
fn convert_messages_to_kimi(messages: Vec<Message>) -> Vec<arkavo_kimi::Message> {
    messages
        .into_iter()
        .map(|msg| {
            let (role, content) = match msg.role {
                Role::System => (arkavo_kimi::Role::System, msg.content),
                Role::User => (arkavo_kimi::Role::User, msg.content),
                Role::Assistant => (arkavo_kimi::Role::Assistant, msg.content),
                Role::Tool => (arkavo_kimi::Role::User, msg.tool_result_as_user_text()),
            };
            arkavo_kimi::Message {
                role,
                content,
                images: msg.images,
            }
        })
        .collect()
}

/// Convert arkavo-kimi stream response to arkavo-llm stream response
fn convert_stream_response(resp: arkavo_kimi::StreamResponse) -> StreamResponse {
    StreamResponse {
        response_items: Vec::new(),
        content: resp.content,
        reasoning_content: resp.reasoning_content,
        done: resp.done,
        inference_timing: None,
    }
}

#[async_trait]
impl Provider for KimiProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        let kimi_messages = convert_messages_to_kimi(messages);

        // Use the arkavo-kimi Provider trait
        use arkavo_kimi::Provider as KimiProviderTrait;

        self.inner
            .complete(kimi_messages)
            .await
            .map_err(|e| match e {
                arkavo_kimi::provider::LlmError::Config(msg) => Error::Config(msg),
                arkavo_kimi::provider::LlmError::Provider(msg) => Error::Provider(msg),
                arkavo_kimi::provider::LlmError::Stream(msg) => Error::Stream(msg),
            })
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let kimi_messages = convert_messages_to_kimi(messages);

        // Use the arkavo-kimi Provider trait
        use arkavo_kimi::Provider as KimiProviderTrait;

        let kimi_stream = self
            .inner
            .stream(kimi_messages)
            .await
            .map_err(|e| match e {
                arkavo_kimi::provider::LlmError::Config(msg) => Error::Config(msg),
                arkavo_kimi::provider::LlmError::Provider(msg) => Error::Provider(msg),
                arkavo_kimi::provider::LlmError::Stream(msg) => Error::Stream(msg),
            })?;

        // Convert the stream
        let mapped_stream = futures::StreamExt::map(kimi_stream, |result| {
            result.map(convert_stream_response).map_err(|e| match e {
                arkavo_kimi::provider::LlmError::Config(msg) => Error::Config(msg),
                arkavo_kimi::provider::LlmError::Provider(msg) => Error::Provider(msg),
                arkavo_kimi::provider::LlmError::Stream(msg) => Error::Stream(msg),
            })
        });

        Ok(Box::new(Box::pin(mapped_stream)))
    }

    fn name(&self) -> &'static str {
        // Use the arkavo-kimi Provider trait
        use arkavo_kimi::Provider as KimiProviderTrait;
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// Moonshot's chat API treats a trailing assistant message as a turn to
    /// continue, so a tool result sent under that role makes the model finish
    /// its own tool output. `arkavo_kimi::Role` has no tool variant, so the
    /// result has to arrive as user text that names the tool.
    #[spec("ASTRA-002")]
    #[test]
    fn tool_result_is_not_sent_as_an_assistant_turn() {
        let messages = vec![
            Message::user("what is the weather in Dublin"),
            Message::assistant_with_tool_calls(
                "Checking the forecast.",
                vec![crate::ToolCall {
                    name: "get_weather".to_string(),
                    arguments: r#"{"location":"Dublin"}"#.to_string(),
                    id: Some("call_1".to_string()),
                }],
            ),
            Message::tool_result("sunny, 21C", "call_1", "get_weather"),
        ];

        let converted = convert_messages_to_kimi(messages);

        let last = converted.last().expect("conversation is not empty");
        assert_ne!(last.role, arkavo_kimi::Role::Assistant);
        assert_eq!(last.role, arkavo_kimi::Role::User);
        assert!(last.content.contains("sunny, 21C"), "{:?}", last.content);
        assert!(last.content.contains("get_weather"), "{:?}", last.content);
    }
}
