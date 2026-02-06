use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use tokio_stream::Stream;
use tracing::{debug, warn};

use super::types::{ChatRequest, ChatResponse};
use crate::{Error, LlmConfig, Message, Provider, Result, StreamResponse};

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
            model: model.unwrap_or_default(), // Empty default, will be selected dynamically
        }
    }

    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("OLLAMA_BASE_URL").ok();
        let model = std::env::var("OLLAMA_MODEL").ok();
        Ok(Self::new(base_url, model))
    }

    pub fn from_config(config: &LlmConfig) -> Result<Self> {
        Ok(Self::new(config.base_url.clone(), config.model.clone()))
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
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
        // If a specific model is set via env var, use it
        if !self.model.is_empty() {
            return Ok(self.model.clone());
        }

        let has_images = messages
            .iter()
            .any(|msg| msg.images.as_ref().is_some_and(|imgs| !imgs.is_empty()));

        if has_images {
            self.select_vision_model().await
        } else {
            self.select_text_model(messages).await
        }
    }

    async fn select_vision_model(&self) -> Result<String> {
        let available_models = self.list_models().await?;

        let vision_models = ["llava:7b", "llava:latest", "llava"];

        for model in &vision_models {
            if available_models.iter().any(|m| m.contains(model)) {
                debug!("Selected vision model: {model}");
                return Ok((*model).to_string());
            }
        }

        warn!(
            "No vision model found, using default model. Install llava with: ollama pull llava:7b"
        );
        Ok(self.model.clone())
    }

    async fn select_text_model(&self, messages: &[Message]) -> Result<String> {
        let available_models = self.list_models().await?;

        let is_coding = messages.iter().any(|msg| {
            let content = msg.content.to_lowercase();
            content.contains("code")
                || content.contains("function")
                || content.contains("class")
                || content.contains("debug")
                || content.contains("implement")
        });

        if is_coding {
            let coding_models = ["devstral:latest", "devstral"];
            for model in &coding_models {
                if available_models.iter().any(|m| m.contains(model)) {
                    debug!("Selected coding model: {model}");
                    return Ok((*model).to_string());
                }
            }
        }

        let general_models = [
            "qwen3:0.6b", // Specifically prefer the 0.6b model
            "qwen3:latest",
            "qwen3",
            "qwen2.5:latest",
            "qwen2.5",
            "qwen:latest",
            "qwen",
            "devstral:latest",
            "devstral",
            "llama3.2:latest",
            "llama3.2",
            "llama3.1:latest",
            "llama3.1",
        ];

        for model in &general_models {
            // For specific version tags like "qwen3:0.6b", do exact matching
            if model.contains(':') {
                if available_models.iter().any(|m| m == model) {
                    debug!("Selected general model: {model}");
                    return Ok((*model).to_string());
                }
            } else {
                // For general model names, use contains matching
                if available_models.iter().any(|m| m.contains(model)) {
                    debug!("Selected general model: {model}");
                    return Ok((*model).to_string());
                }
            }
        }

        // If no preferred model found, use the first available model
        if let Some(first_model) = available_models.first() {
            debug!("Using first available model: {first_model}");
            Ok(first_model.clone())
        } else {
            Err(crate::Error::Provider(
                "No models available. Please run 'ollama pull' to download a model.".to_string(),
            ))
        }
    }
}

#[async_trait]
impl Provider for OllamaClient {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String> {
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
                "Ollama API error: {status} - {text}"
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
                "Ollama API error: {status} - {text}"
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
                                        reasoning_content: None,
                                        done: resp.done,
                                    }));
                                }
                                Err(e) => {
                                    // Try alternative format for streaming responses
                                    #[derive(Deserialize)]
                                    struct StreamingResponse {
                                        response: Option<String>,
                                        done: bool,
                                    }

                                    if let Ok(stream_resp) =
                                        serde_json::from_str::<StreamingResponse>(line)
                                    {
                                        responses.push(Ok(StreamResponse {
                                            content: stream_resp.response.unwrap_or_default(),
                                            reasoning_content: None,
                                            done: stream_resp.done,
                                        }));
                                    } else {
                                        warn!("Failed to parse response: {line}");
                                        responses.push(Err(Error::Json(e)));
                                    }
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

    fn name(&self) -> &'static str {
        "ollama"
    }
}
