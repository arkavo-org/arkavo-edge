use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use tokio_stream::Stream;
use tracing::{debug, warn};

use super::types::{ChatRequest, ChatResponse};
use crate::{Error, Message, Provider, Result, StreamResponse};

#[derive(Debug, Deserialize)]
struct ModelInfo {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    models: Vec<ModelInfo>,
}

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
            model: model.unwrap_or_else(|| "devstral".to_string()),
        }
    }

    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("OLLAMA_BASE_URL").ok();
        let model = std::env::var("OLLAMA_MODEL").ok();
        Ok(Self::new(base_url, model))
    }

    async fn list_models(&self) -> Result<Vec<String>> {
        debug!("Fetching available models from Ollama");
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await?;

        if !response.status().is_success() {
            warn!("Failed to fetch models from Ollama");
            return Ok(vec![]);
        }

        let models_response: ModelsResponse = response.json().await?;
        Ok(models_response.models.into_iter().map(|m| m.name).collect())
    }

    async fn select_model(&self, messages: &[Message]) -> Result<String> {
        let has_images = messages.iter().any(|msg| {
            msg.images.as_ref().is_some_and(|imgs| !imgs.is_empty())
        });

        if has_images {
            self.select_vision_model().await
        } else {
            self.select_text_model(messages).await
        }
    }

    async fn select_vision_model(&self) -> Result<String> {
        let available_models = self.list_models().await?;
        
        let vision_models = [
            "qwen2.5vl:latest",
            "qwen2.5vl",
            "llava:latest",
            "llava",
            "llama3.2-vision:latest",
            "llama3.2-vision",
        ];

        for model in &vision_models {
            if available_models.iter().any(|m| m.contains(model)) {
                debug!("Selected vision model: {}", model);
                return Ok(model.to_string());
            }
        }

        warn!("No vision model found, using default model");
        Ok(self.model.clone())
    }

    async fn select_text_model(&self, messages: &[Message]) -> Result<String> {
        let available_models = self.list_models().await?;
        
        let is_coding = messages.iter().any(|msg| {
            let content = msg.content.to_lowercase();
            content.contains("code") || 
            content.contains("function") || 
            content.contains("class") ||
            content.contains("debug") ||
            content.contains("implement")
        });

        if is_coding {
            let coding_models = ["devstral:latest", "devstral"];
            for model in &coding_models {
                if available_models.iter().any(|m| m.contains(model)) {
                    debug!("Selected coding model: {}", model);
                    return Ok(model.to_string());
                }
            }
        }

        let general_models = [
            "devstral:latest",
            "devstral",
            "llama3.2:latest",
            "llama3.2",
            "llama3.1:latest",
            "llama3.1",
        ];

        for model in &general_models {
            if available_models.iter().any(|m| m.contains(model)) {
                debug!("Selected general model: {}", model);
                return Ok(model.to_string());
            }
        }

        debug!("Using default model: {}", self.model);
        Ok(self.model.clone())
    }
}

#[async_trait]
impl Provider for OllamaClient {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        let model = self.select_model(&messages).await?;
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
        let model = self.select_model(&messages).await?;
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
