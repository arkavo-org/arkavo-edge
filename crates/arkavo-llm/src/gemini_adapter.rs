use crate::error::{Error, Result};
use crate::message::Message;
use crate::provider::Provider;
use crate::stream::StreamResponse;
use arkavo_gemini::{GeminiSseStream, RestClient};
use async_trait::async_trait;
use std::env;
use tokio_stream::StreamExt;

pub struct GeminiProvider {
    client: RestClient,
}

impl GeminiProvider {
    pub fn new() -> Result<Self> {
        let api_key = env::var("GEMINI_API_KEY")
            .map_err(|_| Error::Config("GEMINI_API_KEY not set".into()))?;

        let model = env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-flash-latest".to_string());

        Ok(Self {
            client: RestClient::new(api_key, model),
        })
    }
}

impl Default for GeminiProvider {
    fn default() -> Self {
        Self::new().expect("Failed to create GeminiProvider")
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    fn name(&self) -> &'static str {
        "gemini"
    }

    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        if messages.is_empty() {
            return Err(Error::Provider("No messages provided".into()));
        }

        let last_message = messages
            .last()
            .ok_or_else(|| Error::Provider("No messages provided".into()))?;

        let prompt = &last_message.content;

        let (text, _function_calls) = self
            .client
            .generate_content(prompt, None)
            .await
            .map_err(|e| Error::Provider(format!("Gemini API error: {e}")))?;

        text.ok_or_else(|| Error::Provider("No text response from Gemini".into()))
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn tokio_stream::Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        if messages.is_empty() {
            return Err(Error::Provider("No messages provided".into()));
        }

        let last_message = messages
            .last()
            .ok_or_else(|| Error::Provider("No messages provided".into()))?;

        let prompt = &last_message.content;

        let gemini_stream: GeminiSseStream = self
            .client
            .stream_generate_content(prompt, None)
            .await
            .map_err(|e| Error::Stream(format!("Gemini streaming error: {e}")))?;

        let adapter_stream = gemini_stream.map(|result| {
            result
                .map(|response| StreamResponse {
                    content: response.text.unwrap_or_default(),
                    done: response.done,
                })
                .map_err(|e| Error::Stream(format!("Stream error: {e}")))
        });

        Ok(Box::new(Box::pin(adapter_stream)))
    }
}
