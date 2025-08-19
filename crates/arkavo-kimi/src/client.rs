use crate::error::{KimiError, Result};
use crate::provider::{Message, Role};
use crate::retry::{RetryClient, RetryConfig};
use crate::stream::SseParser;
use crate::types::{
    ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ChatRole, ErrorResponse, Model,
    Tool, ToolChoice,
};
use reqwest::Client;
use std::time::Duration;
use tracing::{debug, info};

/// Kimi API configuration
#[derive(Clone, Debug)]
pub struct KimiConfig {
    /// API key for authentication
    pub api_key: String,
    /// Base URL (default: https://api.moonshot.ai/v1)
    pub base_url: String,
    /// Model to use
    pub model: Model,
    /// Request timeout
    pub timeout: Duration,
    /// Retry configuration
    pub retry_config: RetryConfig,
    /// Buffer size for SSE streaming (default: 1024)
    pub buffer_size: usize,
}

impl Default for KimiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.moonshot.ai/v1".to_string(),
            model: Model::MoonshotV1_8k,
            timeout: Duration::from_secs(60),
            retry_config: RetryConfig::default(),
            buffer_size: 1024,
        }
    }
}

impl KimiConfig {
    /// Create config from environment variables
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("MOONSHOT_API_KEY")
            .map_err(|_| KimiError::Config("MOONSHOT_API_KEY not set".to_string()))?;

        // Validate API key format
        if api_key.trim().is_empty() {
            return Err(KimiError::Config("MOONSHOT_API_KEY is empty".to_string()));
        }
        if api_key.len() < 10 {
            return Err(KimiError::Config(
                "MOONSHOT_API_KEY appears to be invalid (too short)".to_string(),
            ));
        }

        let base_url = std::env::var("KIMI_API_BASE")
            .unwrap_or_else(|_| "https://api.moonshot.ai/v1".to_string());

        let model = std::env::var("KIMI_MODEL")
            .ok()
            .and_then(|m| match m.as_str() {
                "moonshot-v1-8k" => Some(Model::MoonshotV1_8k),
                "moonshot-v1-32k" => Some(Model::MoonshotV1_32k),
                "moonshot-v1-128k" => Some(Model::MoonshotV1_128k),
                _ => None,
            })
            .unwrap_or(Model::MoonshotV1_8k);

        Ok(Self {
            api_key,
            base_url,
            model,
            ..Default::default()
        })
    }

    /// Set the buffer size for SSE streaming
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }
}

/// Native Kimi API client
pub struct KimiClient {
    config: KimiConfig,
    http_client: Client,
    retry_client: RetryClient,
}

impl KimiClient {
    /// Create a new Kimi client
    pub fn new(config: KimiConfig) -> Result<Self> {
        let http_client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| KimiError::Config(format!("Failed to build HTTP client: {e}")))?;

        let retry_client = RetryClient::new(config.retry_config.clone());

        Ok(Self {
            config,
            http_client,
            retry_client,
        })
    }

    pub fn config(&self) -> &KimiConfig {
        &self.config
    }

    /// Convert arkavo-llm messages to Kimi API format
    fn convert_messages(&self, messages: Vec<Message>) -> Vec<ChatMessage> {
        messages
            .into_iter()
            .map(|msg| ChatMessage {
                role: match msg.role {
                    Role::System => ChatRole::System,
                    Role::User => ChatRole::User,
                    Role::Assistant => ChatRole::Assistant,
                },
                content: msg.content,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            })
            .collect()
    }

    /// Create a chat completion request
    pub async fn create_chat_completion(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<String> {
        let request = ChatCompletionRequest {
            model: self.config.model.as_str().to_string(),
            messages: self.convert_messages(messages),
            tools,
            tool_choice,
            temperature,
            top_p,
            max_tokens: None,
            stream: Some(false),
            presence_penalty: None,
            frequency_penalty: None,
            n: None,
            stop: None,
        };

        let url = format!("{}/chat/completions", self.config.base_url);

        debug!("Sending request to Kimi API: {}", url);

        let response = self
            .retry_client
            .execute_with_retry(|| {
                let client = self.http_client.clone();
                let url = url.clone();
                let api_key = self.config.api_key.clone();
                let request = request.clone();
                async move {
                    let resp = client
                        .post(&url)
                        .header("Authorization", format!("Bearer {api_key}"))
                        .header("Content-Type", "application/json")
                        .json(&request)
                        .send()
                        .await?;

                    if resp.status().is_success() {
                        let completion: ChatCompletionResponse = resp.json().await?;
                        completion
                            .choices
                            .first()
                            .map(|choice| choice.message.content.clone())
                            .ok_or_else(|| {
                                KimiError::Other(anyhow::anyhow!(
                                    "Empty choices array in Kimi API response"
                                ))
                            })
                    } else {
                        let status = resp.status();
                        let headers = resp.headers().clone();

                        // Try to parse error response
                        if let Ok(error_resp) = resp.json::<ErrorResponse>().await {
                            let error = &error_resp.error;
                            match status {
                                reqwest::StatusCode::TOO_MANY_REQUESTS => {
                                    Err(KimiError::rate_limited_from_headers(
                                        &headers,
                                        Some(error.message.clone()),
                                    ))
                                }
                                _ => Err(KimiError::from_status(status, error.message.clone())),
                            }
                        } else {
                            Err(KimiError::from_status(
                                status,
                                format!("HTTP error: {status}"),
                            ))
                        }
                    }
                }
            })
            .await?;

        Ok(response)
    }

    /// Create a streaming chat completion
    pub async fn create_chat_completion_stream(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<SseParser> {
        let request = ChatCompletionRequest {
            model: self.config.model.as_str().to_string(),
            messages: self.convert_messages(messages),
            tools,
            tool_choice,
            temperature,
            top_p,
            max_tokens: None,
            stream: Some(true),
            presence_penalty: None,
            frequency_penalty: None,
            n: None,
            stop: None,
        };

        let url = format!("{}/chat/completions", self.config.base_url);

        info!("Creating streaming request to Kimi API");

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read error response".to_string());
            return Err(KimiError::from_status(status, error_text));
        }

        Ok(SseParser::new(
            response.bytes_stream(),
            self.config.buffer_size,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_with_buffer_size() {
        let config = KimiConfig::default().with_buffer_size(2048);

        assert_eq!(config.buffer_size, 2048);
    }

    #[test]
    fn test_config_default_buffer_size() {
        let config = KimiConfig::default();
        assert_eq!(config.buffer_size, 1024);
    }
}
