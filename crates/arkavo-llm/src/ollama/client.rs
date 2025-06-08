use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use tokio_stream::Stream;
use tracing::debug;

use super::types::{ChatRequest, ChatResponse};
use crate::{Error, Message, Provider, Result, StreamResponse};

pub struct OllamaClient {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaClient {
    pub fn new(base_url: Option<String>, model: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
            model: model.unwrap_or_else(|| "llama3.2".to_string()),
        }
    }

    fn select_model(&self, messages: &[Message]) -> String {
        let has_images = messages.iter().any(|msg| {
            msg.images.as_ref().is_some_and(|imgs| !imgs.is_empty())
        });

        if has_images {
            // Auto-select vision model based on availability preference
            self.select_vision_model()
        } else {
            self.model.clone()
        }
    }

    fn select_vision_model(&self) -> String {
        // Priority order for vision models
        let vision_models = [
            "qwen2.5vl:7b",
            "qwen2.5vl:3b", 
            "qwen2.5vl:32b",
            "qwen2.5vl:72b",
            "llama3.2-vision:11b",
            "llama3.2-vision:90b",
            "llava:7b",
            "llava:13b",
            "llava:34b",
        ];

        // For now, default to qwen2.5vl:7b as it's the most capable mid-size model
        // In production, this could query available models from Ollama API
        vision_models[0].to_string()
    }

    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("OLLAMA_BASE_URL").ok();
        let model = std::env::var("OLLAMA_MODEL").ok();
        Ok(Self::new(base_url, model))
    }
}

#[async_trait]
impl Provider for OllamaClient {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        let model = self.select_model(&messages);
        let request = ChatRequest {
            model,
            messages,
            stream: false,
        };

        debug!("Sending chat request to Ollama");
        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "Ollama API error: {} - {}",
                status, text
            )));
        }

        let chat_response: ChatResponse = response.json().await?;
        Ok(chat_response.message.content)
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let model = self.select_model(&messages);
        let request = ChatRequest {
            model,
            messages,
            stream: true,
        };

        debug!("Sending streaming chat request to Ollama");
        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "Ollama API error: {} - {}",
                status, text
            )));
        }

        let stream = response
            .bytes_stream()
            .map(move |chunk| {
                match chunk {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);

                        // Parse JSON lines
                        let mut responses = Vec::new();
                        for line in text.lines() {
                            if line.trim().is_empty() {
                                continue;
                            }

                            match serde_json::from_str::<ChatResponse>(line) {
                                Ok(resp) => {
                                    responses.push(Ok(StreamResponse {
                                        content: resp.message.content,
                                        done: resp.done,
                                    }));
                                }
                                Err(e) => {
                                    responses.push(Err(Error::Json(e)));
                                }
                            }
                        }

                        stream::iter(responses)
                    }
                    Err(e) => stream::iter(vec![Err(Error::Request(e))]),
                }
            })
            .flatten();

        Ok(Box::new(Box::pin(stream)))
    }

    fn name(&self) -> &str {
        "ollama"
    }
}
