use crate::client::{QwenClient, QwenConfig};
use crate::error::{QwenError, Result};
use crate::models::QwenModel;
use crate::stream::StreamResponse;
use crate::types::{ChatMessage, MessageContent, Role};
use crate::vision::create_image_part;
use async_trait::async_trait;
use futures::Stream;

pub type LlmError = QwenError;
pub type LlmResult<T> = Result<T>;

#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, messages: Vec<Message>) -> LlmResult<String>;
    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> LlmResult<Box<dyn Stream<Item = LlmResult<StreamResponse>> + Send + Unpin>>;
    fn name(&self) -> &'static str;
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

pub struct QwenProvider {
    client: QwenClient,
    model: QwenModel,
}

impl QwenProvider {
    pub fn new(config: QwenConfig) -> LlmResult<Self> {
        let model_str = config.model.clone();
        let client = QwenClient::new(config)?;
        let model = QwenModel::parse(&model_str);
        Ok(Self { client, model })
    }

    pub fn from_env() -> LlmResult<Self> {
        let client = QwenClient::from_env()?;
        let model = QwenModel::parse(&client.config().model);
        Ok(Self { client, model })
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.client = self.client.with_temperature(temperature);
        self
    }

    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.client = self.client.with_top_p(top_p);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.client = self.client.with_max_tokens(max_tokens);
        self
    }

    pub fn client(&self) -> &QwenClient {
        &self.client
    }

    fn convert_messages(&self, messages: Vec<Message>) -> LlmResult<Vec<ChatMessage>> {
        messages
            .into_iter()
            .map(|msg| self.convert_message(msg))
            .collect()
    }

    fn convert_message(&self, msg: Message) -> LlmResult<ChatMessage> {
        let role = match msg.role {
            MessageRole::System => Role::System,
            MessageRole::User => Role::User,
            MessageRole::Assistant => Role::Assistant,
        };

        let content = if let Some(images) = msg.images {
            if !self.model.supports_vision() {
                return Err(QwenError::VisionNotSupported {
                    model: self.model.to_string(),
                });
            }

            let mut parts = vec![];
            if !msg.content.is_empty() {
                parts.push(crate::types::ContentPart::Text { text: msg.content });
            }

            for image in images {
                parts.push(create_image_part(&image));
            }

            MessageContent::Parts(parts)
        } else {
            MessageContent::Text(msg.content)
        };

        Ok(ChatMessage {
            role,
            content,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        })
    }
}

#[async_trait]
impl Provider for QwenProvider {
    async fn complete(&self, messages: Vec<Message>) -> LlmResult<String> {
        let qwen_messages = self.convert_messages(messages)?;

        let response = self.client.complete(qwen_messages, None, None).await?;

        if let Some(choice) = response.choices.first() {
            match &choice.message.content {
                MessageContent::Text(text) => Ok(text.clone()),
                MessageContent::Parts(parts) => {
                    let text = parts
                        .iter()
                        .filter_map(|p| match p {
                            crate::types::ContentPart::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    Ok(text)
                }
            }
        } else {
            Err(QwenError::InternalError {
                message: "No choices in response".to_string(),
            })
        }
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> LlmResult<Box<dyn Stream<Item = LlmResult<StreamResponse>> + Send + Unpin>> {
        let qwen_messages = self.convert_messages(messages)?;

        let stream = self.client.stream(qwen_messages, None, None).await?;

        Ok(Box::new(Box::pin(stream)))
    }

    fn name(&self) -> &'static str {
        "qwen"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::QwenRegion;

    #[test]
    fn test_message_conversion_text_only() {
        let config = QwenConfig {
            api_key: "test_key".to_string(),
            base_url: QwenRegion::International.base_url().to_string(),
            model: "qwen3-32b".to_string(),
            region: QwenRegion::International,
            ..Default::default()
        };

        let provider = QwenProvider::new(config).unwrap();

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
        ];

        let converted = provider.convert_messages(messages).unwrap();
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].role, Role::System);
        assert_eq!(converted[1].role, Role::User);
    }

    #[test]
    fn test_message_conversion_with_vision() {
        let config = QwenConfig {
            api_key: "test_key".to_string(),
            base_url: QwenRegion::International.base_url().to_string(),
            model: "qwen-vl-max".to_string(),
            region: QwenRegion::International,
            ..Default::default()
        };

        let provider = QwenProvider::new(config).unwrap();

        let messages = vec![Message {
            role: MessageRole::User,
            content: "What's in this image?".to_string(),
            images: Some(vec!["iVBORw0KGg==".to_string()]),
        }];

        let converted = provider.convert_messages(messages).unwrap();
        assert_eq!(converted.len(), 1);

        match &converted[0].content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
            }
            _ => panic!("Expected Parts content"),
        }
    }

    #[test]
    fn test_vision_not_supported_error() {
        let config = QwenConfig {
            api_key: "test_key".to_string(),
            base_url: QwenRegion::International.base_url().to_string(),
            model: "qwen3-32b".to_string(),
            region: QwenRegion::International,
            ..Default::default()
        };

        let provider = QwenProvider::new(config).unwrap();

        let messages = vec![Message {
            role: MessageRole::User,
            content: "What's in this image?".to_string(),
            images: Some(vec!["iVBORw0KGg==".to_string()]),
        }];

        let result = provider.convert_messages(messages);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            QwenError::VisionNotSupported { .. }
        ));
    }

    #[test]
    fn test_builder_pattern() {
        let config = QwenConfig::default();
        let provider = QwenProvider::new(config)
            .unwrap()
            .with_temperature(0.8)
            .with_top_p(0.9)
            .with_max_tokens(2048);

        assert_eq!(provider.client.config().temperature, Some(0.8));
        assert_eq!(provider.client.config().top_p, Some(0.9));
        assert_eq!(provider.client.config().max_tokens, Some(2048));
    }
}
