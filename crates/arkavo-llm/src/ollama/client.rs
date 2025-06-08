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
            model: model.unwrap_or_else(|| "devstral:latest".to_string()),
        }
    }

    async fn get_available_models(&self) -> Result<Vec<String>> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Error::Provider("Failed to fetch available models".to_string()));
        }

        let models_response: serde_json::Value = response.json().await?;
        let models = models_response["models"]
            .as_array()
            .ok_or_else(|| Error::Provider("Invalid models response format".to_string()))?
            .iter()
            .filter_map(|model| model["name"].as_str())
            .map(|name| name.to_string())
            .collect();

        Ok(models)
    }

    async fn select_best_available_model(&self, preferred_models: &[&str]) -> Result<String> {
        let available_models = self.get_available_models().await?;
        
        // Find first preferred model that's available
        for preferred in preferred_models {
            if available_models.iter().any(|available| {
                available == preferred || available.starts_with(&format!("{}:", preferred))
            }) {
                return Ok(preferred.to_string());
            }
        }

        // Fallback to first available model
        if let Some(first_available) = available_models.first() {
            return Ok(first_available.clone());
        }

        Err(Error::Provider("No models available. Please install a model with 'ollama pull <model>'".to_string()))
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

    async fn select_text_model(&self, messages: &[Message]) -> Result<String> {
        let content = messages.iter()
            .map(|msg| msg.content.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();

        // Detect coding context
        let coding_keywords = [
            "code", "function", "variable", "class", "method", "implementation",
            "debug", "error", "bug", "refactor", "optimize", "algorithm",
            "programming", "rust", "python", "javascript", "typescript",
            "api", "endpoint", "database", "sql", "json", "xml", "html", "css",
            "test", "unit test", "integration", "compile", "build", "deploy"
        ];

        let is_coding_task = coding_keywords.iter().any(|keyword| content.contains(keyword));

        if is_coding_task {
            self.select_coding_model().await
        } else {
            // Default to devstral for general tasks too (it's a good general purpose model)
            Ok(self.model.clone())
        }
    }

    async fn select_coding_model(&self) -> Result<String> {
        let preferred_coding_models = [
            "devstral:latest",
            "devstral",
            "deepseek-r1:14b", 
            "deepseek-r1",
            "qwen2.5:14b",
            "qwen2.5:7b",
            "qwen2.5",
            "llama3.2:3b",
            "llama3.2",
        ];

        self.select_best_available_model(&preferred_coding_models).await
    }

    async fn select_vision_model(&self) -> Result<String> {
        let preferred_vision_models = [
            "qwen2.5vl:latest",
            "qwen2.5vl:7b",
            "qwen2.5vl:3b", 
            "qwen2.5vl:32b",
            "qwen2.5vl:72b",
            "qwen2.5vl",
            "llama3.2-vision:11b",
            "llama3.2-vision:90b",
            "llama3.2-vision",
            "llava:7b",
            "llava:13b", 
            "llava:34b",
            "llava",
        ];

        self.select_best_available_model(&preferred_vision_models).await
    }

    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("OLLAMA_BASE_URL").ok();
        let model = std::env::var("OLLAMA_MODEL").ok();
        Ok(Self::new(base_url, model))
    }

    pub async fn from_env_with_discovery() -> Result<Self> {
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let model = std::env::var("OLLAMA_MODEL").ok();

        let client = Self::new(Some(base_url), model);
        
        // Test if we can connect and get available models
        match client.get_available_models().await {
            Ok(models) => {
                if models.is_empty() {
                    return Err(Error::Provider("No models available. Please install a model with 'ollama pull <model>'".to_string()));
                }
                // Update default model to first available if not explicitly set
                if client.model == "devstral:latest" && !models.iter().any(|m| m == "devstral:latest" || m.starts_with("devstral:")) {
                    let mut updated_client = client;
                    updated_client.model = models[0].clone();
                    Ok(updated_client)
                } else {
                    Ok(client)
                }
            }
            Err(_) => {
                // Fallback to basic client if we can't connect (for backwards compatibility)
                Ok(client)
            }
        }
    }
}

#[async_trait]
impl Provider for OllamaClient {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        let model = self.select_model(&messages).await?;
        debug!("Selected model: {}", model);
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
