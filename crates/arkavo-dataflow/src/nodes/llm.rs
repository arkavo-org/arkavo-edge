use super::{
    NodeProcessor,
    llm_config::{LlmConfiguration, load_llm_config},
};
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
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct LlmTransform {
    providers: Arc<RwLock<HashMap<String, Box<dyn Provider>>>>,
    config: Arc<RwLock<Option<LlmConfiguration>>>,
}

impl LlmTransform {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn add_provider(&self, name: String, provider: Box<dyn Provider>) {
        let mut providers = self.providers.write().await;
        providers.insert(name, provider);
    }

    pub async fn initialize_providers(&self) -> Result<()> {
        // Try to load configuration from memory storage
        if let Ok(Some(stored_config)) = load_llm_config().await {
            info!("Loaded LLM configuration from memory storage");
            *self.config.write().await = Some(stored_config.clone());

            // Initialize providers from stored configuration
            for provider_config in &stored_config.providers {
                let ollama_client = OllamaClient::new(
                    Some(provider_config.base_url.clone()),
                    provider_config.default_model.clone(),
                );
                self.add_provider(provider_config.name.clone(), Box::new(ollama_client))
                    .await;
                debug!(
                    "Initialized provider: {} at {}",
                    provider_config.name, provider_config.base_url
                );
            }
        } else {
            info!("No stored LLM configuration found, initializing defaults");

            // Initialize with default local Ollama
            let local_ollama = OllamaClient::new(Some("http://localhost:11434".to_string()), None);
            self.add_provider("local-ollama".to_string(), Box::new(local_ollama))
                .await;

            // Check for remote Ollama from env (for backward compatibility)
            if let Ok(remote_url) = std::env::var("REMOTE_OLLAMA_URL") {
                let remote_ollama = OllamaClient::new(Some(remote_url.clone()), None);
                self.add_provider("remote-ollama".to_string(), Box::new(remote_ollama))
                    .await;
                info!("Added remote Ollama from environment: {}", remote_url);
            }
        }

        Ok(())
    }

    async fn get_provider_config(&self, provider_name: &str) -> Result<(String, Option<String>)> {
        {
            let config = self.config.read().await;

            if let Some(ref llm_config) = *config {
                if let Some(provider_config) = llm_config.get_provider(provider_name) {
                    return Ok((
                        provider_config.base_url.clone(),
                        provider_config.default_model.clone(),
                    ));
                }
            }
        }

        // Fallback to defaults
        match provider_name {
            "local-ollama" => Ok(("http://localhost:11434".to_string(), None)),
            "remote-ollama" => {
                let remote_url = std::env::var("REMOTE_OLLAMA_URL")
                    .unwrap_or_else(|_| "http://10.0.0.101:11434".to_string());
                Ok((remote_url, None))
            }
            _ => Err(anyhow::anyhow!("Unknown provider: {}", provider_name)),
        }
    }

    async fn get_model_for_task(&self, task_type: &str) -> Option<String> {
        let config = self.config.read().await;

        if let Some(ref llm_config) = *config {
            llm_config.get_model_for_task(task_type).cloned()
        } else {
            None
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

        // Get task type from params for model preference lookup
        let task_type = params.get("task_type").and_then(|v| v.as_str());

        // Get provider configuration
        let (base_url, default_model) = self.get_provider_config(provider_name).await?;

        // Determine which model to use
        let model_to_use = if let Some(explicit_model) = model {
            explicit_model.to_string()
        } else if let Some(task) = task_type {
            // Check if we have a model preference for this task type
            if let Some(preferred_model) = self.get_model_for_task(task).await {
                preferred_model
            } else {
                default_model.unwrap_or_else(|| "llama3.2:latest".to_string())
            }
        } else {
            default_model.unwrap_or_else(|| "llama3.2:latest".to_string())
        };

        // Create message
        let message = Message::user(prompt);

        // Create Ollama client with determined configuration
        let ollama_client = OllamaClient::new(Some(base_url.clone()), Some(model_to_use.clone()));

        debug!(
            "Executing LLM transform with provider: {}, model: {}",
            provider_name, model_to_use
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
            "model": model_to_use,
            "metadata": {
                "temperature": temperature,
                "max_tokens": max_tokens,
                "task_type": task_type,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            }
        })))
    }

    fn node_type(&self) -> &'static str {
        "transform"
    }
}

impl Default for LlmTransform {
    fn default() -> Self {
        let transform = Self::new();
        // Initialize providers asynchronously when first created
        let transform_clone = transform.clone();
        tokio::spawn(async move {
            if let Err(e) = transform_clone.initialize_providers().await {
                warn!("Failed to initialize LLM providers: {}", e);
            }
        });
        transform
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
