use crate::{Message, Provider, Result, Role, StreamResponse};
use arkavo_qwen::Provider as QwenProviderTrait;
use async_trait::async_trait;
use tokio_stream::Stream;

pub struct QwenProvider {
    inner: arkavo_qwen::QwenProvider,
}

impl QwenProvider {
    pub fn new(config: arkavo_qwen::QwenConfig) -> Result<Self> {
        let inner = arkavo_qwen::QwenProvider::new(config)
            .map_err(|e| crate::Error::Provider(e.to_string()))?;
        Ok(Self { inner })
    }

    pub fn from_env() -> Result<Self> {
        let inner = arkavo_qwen::QwenProvider::from_env()
            .map_err(|e| crate::Error::Provider(e.to_string()))?;
        Ok(Self { inner })
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.inner = self.inner.with_temperature(temperature);
        self
    }

    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.inner = self.inner.with_top_p(top_p);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.inner = self.inner.with_max_tokens(max_tokens);
        self
    }
}

fn convert_message(msg: Message) -> arkavo_qwen::Message {
    let role = match msg.role {
        Role::System => arkavo_qwen::MessageRole::System,
        Role::User => arkavo_qwen::MessageRole::User,
        Role::Assistant => arkavo_qwen::MessageRole::Assistant,
    };

    arkavo_qwen::Message {
        role,
        content: msg.content,
        images: msg.images,
    }
}

fn convert_stream_response(resp: arkavo_qwen::StreamResponse) -> crate::StreamResponse {
    crate::StreamResponse {
        content: resp.content,
        // Qwen stream payload currently emits only content/tool-calls deltas.
        // Keep this as None until arkavo-qwen exposes a reasoning channel.
        reasoning_content: None,
        done: resp.done,
    }
}

#[async_trait]
impl Provider for QwenProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
        let qwen_messages: Vec<arkavo_qwen::Message> =
            messages.into_iter().map(convert_message).collect();

        self.inner
            .complete(qwen_messages)
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let qwen_messages: Vec<arkavo_qwen::Message> =
            messages.into_iter().map(convert_message).collect();

        let stream = self
            .inner
            .stream(qwen_messages)
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        let mapped_stream = futures::StreamExt::map(stream, |result| {
            result
                .map(convert_stream_response)
                .map_err(|e| crate::Error::Stream(e.to_string()))
        });

        Ok(Box::new(futures::stream::StreamExt::boxed(mapped_stream)))
    }

    fn name(&self) -> &'static str {
        "qwen"
    }
}

#[cfg(test)]
mod tests {
    use super::convert_message;
    use crate::Message;

    #[test]
    fn test_message_conversion() {
        let msg = Message::user("Hello");
        let qwen_msg = convert_message(msg);
        assert_eq!(qwen_msg.content, "Hello");
        assert!(matches!(qwen_msg.role, arkavo_qwen::MessageRole::User));
    }

    #[test]
    fn test_message_with_images() {
        let msg = Message::user_with_images("Describe this", vec!["img1".to_string()]);
        let qwen_msg = convert_message(msg);
        assert_eq!(qwen_msg.content, "Describe this");
        assert_eq!(qwen_msg.images, Some(vec!["img1".to_string()]));
    }
}
