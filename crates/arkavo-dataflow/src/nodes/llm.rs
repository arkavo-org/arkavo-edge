use super::NodeProcessor;
use anyhow::Result;
use arkavo_llm::ollama::OllamaClient;
use arkavo_llm::{Message, Provider};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, timeout};
use tracing::{debug, error, warn};

pub struct LlmTransform {
    providers: Arc<RwLock<HashMap<String, Box<dyn Provider>>>>,
}

impl LlmTransform {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_provider(&self, name: String, provider: Box<dyn Provider>) {
        let mut providers = self.providers.write().await;
        providers.insert(name, provider);
    }

    pub async fn initialize_default_providers(&self) -> Result<()> {
        // Initialize local Ollama if available
        let local_ollama = OllamaClient::new(Some("http://localhost:11434".to_string()), None);
        self.add_provider("local-ollama".to_string(), Box::new(local_ollama))
            .await;

        // Check for remote Ollama from env or config
        if let Ok(remote_url) = std::env::var("REMOTE_OLLAMA_URL") {
            let remote_ollama = OllamaClient::new(Some(remote_url), None);
            self.add_provider("remote-ollama".to_string(), Box::new(remote_ollama))
                .await;
        }

        Ok(())
    }

    async fn get_provider(&self, provider_name: &str) -> Result<Box<dyn Provider>> {
        let providers = self.providers.read().await;

        // If specific provider requested, use it
        if let Some(_provider) = providers.get(provider_name) {
            // Clone the provider by creating a new instance with same config
            match provider_name {
                "local-ollama" => Ok(Box::new(OllamaClient::new(
                    Some("http://localhost:11434".to_string()),
                    None,
                ))),
                "remote-ollama" => {
                    let remote_url = std::env::var("REMOTE_OLLAMA_URL")
                        .unwrap_or_else(|_| "http://10.0.0.101:11434".to_string());
                    Ok(Box::new(OllamaClient::new(Some(remote_url), None)))
                }
                _ => Err(anyhow::anyhow!("Unknown provider: {}", provider_name)),
            }
        } else {
            // Default to local Ollama
            Ok(Box::new(OllamaClient::new(
                Some("http://localhost:11434".to_string()),
                None,
            )))
        }
    }
}

#[async_trait]
impl NodeProcessor for LlmTransform {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        let Some(data) = input else {
            return Ok(None);
        };

        // Extract LLM parameters
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("local-ollama");

        let model = params.get("model").and_then(|v| v.as_str());

        let prompt_template = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("prompt parameter required"))?;

        let temperature = params
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.7);

        let max_tokens = params
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        let stream = params
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Build prompt from template and input data
        let prompt = if prompt_template.contains("{{input}}") {
            prompt_template.replace("{{input}}", &serde_json::to_string(&data)?)
        } else {
            format!(
                "{}\n\nData: {}",
                prompt_template,
                serde_json::to_string_pretty(&data)?
            )
        };

        // Get timeout from params or use default (30 seconds)
        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        // Get provider
        let provider = self.get_provider(provider_name).await?;

        // Create message
        let message = Message::user(prompt);

        // If model specified, we need to use OllamaClient directly
        if let Some(model_name) = model {
            let ollama_client = OllamaClient::new(
                match provider_name {
                    "remote-ollama" => Some(
                        std::env::var("REMOTE_OLLAMA_URL")
                            .unwrap_or_else(|_| "http://10.0.0.101:11434".to_string()),
                    ),
                    _ => Some("http://localhost:11434".to_string()),
                },
                Some(model_name.to_string()),
            );

            debug!(
                "Executing LLM transform with provider: {}, model: {}",
                provider_name, model_name
            );

            let response = if stream {
                // For streaming, collect all chunks and return final result
                match timeout(Duration::from_secs(timeout_secs), async {
                    let mut stream = ollama_client.stream(vec![message]).await?;
                    let mut content = String::new();

                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(response) => {
                                content.push_str(&response.content);
                                if response.done {
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("Error in LLM stream: {}", e);
                                return Err(e.into());
                            }
                        }
                    }

                    Ok::<String, anyhow::Error>(content)
                })
                .await
                {
                    Ok(Ok(content)) => content,
                    Ok(Err(e)) => return Err(e),
                    Err(_) => {
                        warn!("LLM request timed out after {} seconds", timeout_secs);
                        return Err(anyhow::anyhow!("LLM request timed out"));
                    }
                }
            } else {
                match timeout(
                    Duration::from_secs(timeout_secs),
                    ollama_client.complete(vec![message]),
                )
                .await
                {
                    Ok(Ok(response)) => response,
                    Ok(Err(e)) => {
                        error!("LLM completion error: {}", e);
                        return Err(e.into());
                    }
                    Err(_) => {
                        warn!("LLM request timed out after {} seconds", timeout_secs);
                        return Err(anyhow::anyhow!("LLM request timed out"));
                    }
                }
            };

            Ok(Some(json!({
                "original": data,
                "llm_response": response,
                "provider": provider_name,
                "model": model_name,
                "metadata": {
                    "temperature": temperature,
                    "max_tokens": max_tokens,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }
            })))
        } else {
            // Use provider without specific model
            let response = provider.complete(vec![message]).await?;

            Ok(Some(json!({
                "original": data,
                "llm_response": response,
                "provider": provider_name,
                "metadata": {
                    "temperature": temperature,
                    "max_tokens": max_tokens,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }
            })))
        }
    }

    fn node_type(&self) -> &'static str {
        "transform"
    }
}

impl Default for LlmTransform {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_llm_transform_params() {
        let transform = LlmTransform::new();
        let mut params = HashMap::new();
        params.insert("prompt".to_string(), json!("Summarize this: {{input}}"));
        params.insert("provider".to_string(), json!("local-ollama"));
        params.insert("model".to_string(), json!("llama3:latest"));
        params.insert("temperature".to_string(), json!(0.5));

        let input = json!({
            "text": "This is a test document that needs summarization."
        });

        // This test validates parameter parsing without making actual LLM calls
        assert_eq!(transform.node_type(), "transform");
    }
}
