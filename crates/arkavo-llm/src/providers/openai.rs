use crate::common::{HttpClientBuilder, HttpClientConfig, RetryableHttpClient};
use crate::common::{ProviderError, ProviderResult};
use crate::{Message, Provider, Role, StreamResponse};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// OpenAI API configuration
#[derive(Clone, Debug)]
pub struct OpenAIConfig {
    /// API key for authentication
    pub api_key: String,
    /// Base URL (for OpenAI or Azure endpoints)
    pub base_url: String,
    /// Model to use
    pub model: String,
    /// Organization ID (optional)
    pub organization_id: Option<String>,
    /// API version (for Azure)
    pub api_version: Option<String>,
    /// Whether this is an Azure endpoint
    pub is_azure: bool,
}

impl Default for OpenAIConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4".to_string(),
            organization_id: None,
            api_version: None,
            is_azure: false,
        }
    }
}

/// OpenAI API request structures
#[derive(Debug, Clone, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Choice {
    message: ApiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[allow(clippy::struct_field_names)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

/// Streaming response structures
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

/// Error response structure
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ErrorDetail {
    message: String,
    #[serde(rename = "type")]
    error_type: Option<String>,
    code: Option<String>,
}

/// OpenAI provider implementation
pub struct OpenAIProvider {
    config: OpenAIConfig,
    client: Arc<RetryableHttpClient>,
}

impl OpenAIProvider {
    pub fn new(config: OpenAIConfig) -> ProviderResult<Self> {
        // Validate base URL
        url::Url::parse(&config.base_url)
            .map_err(|e| anyhow::anyhow!("Invalid base URL '{}': {}", config.base_url, e))?;

        let http_config = HttpClientConfig {
            base_url: config.base_url.clone(),
            auth_token: Some(config.api_key.clone()),
            timeout_secs: 60,
            max_retries: 3,
            initial_retry_delay_ms: 1000,
            backoff_factor: 2.0,
            max_retry_delay_ms: 30000,
            jitter_factor: 0.1,
            ..Default::default()
        };

        let builder = HttpClientBuilder::new(http_config);
        let client = Arc::new(RetryableHttpClient::new(builder)?);

        Ok(Self { config, client })
    }

    /// Convert internal messages to API format
    fn convert_messages(&self, messages: Vec<Message>) -> Vec<ApiMessage> {
        messages
            .into_iter()
            .map(|msg| ApiMessage {
                role: match msg.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                },
                content: msg.content,
            })
            .collect()
    }

    /// Build the API endpoint URL
    fn build_url(&self, endpoint: &str) -> String {
        if self.config.is_azure {
            // Azure OpenAI endpoint format
            format!(
                "{}/openai/deployments/{}/{}?api-version={}",
                self.config.base_url,
                self.config.model,
                endpoint,
                self.config.api_version.as_deref().unwrap_or("2024-02-01")
            )
        } else {
            // Standard OpenAI endpoint
            format!("{}/{}", self.config.base_url, endpoint)
        }
    }

    /// Handle API errors
    async fn handle_error_response(&self, response: reqwest::Response) -> ProviderError {
        let status = response.status();
        let headers = response.headers().clone();

        // Try to parse error body
        if let Ok(error_response) = response.json::<ErrorResponse>().await {
            let error = &error_response.error;

            match status {
                StatusCode::TOO_MANY_REQUESTS => {
                    ProviderError::rate_limited_from_headers(&headers, Some(error.message.clone()))
                }
                StatusCode::UNAUTHORIZED => ProviderError::AuthenticationFailed {
                    message: error.message.clone(),
                    provider: "openai".to_string(),
                },
                StatusCode::NOT_FOUND => {
                    if error.message.contains("model") {
                        ProviderError::ModelNotFound {
                            model: self.config.model.clone(),
                            provider: "openai".to_string(),
                            available_models: None,
                        }
                    } else {
                        ProviderError::InvalidRequest {
                            message: error.message.clone(),
                            details: None,
                        }
                    }
                }
                StatusCode::BAD_REQUEST => ProviderError::InvalidRequest {
                    message: error.message.clone(),
                    details: None,
                },
                _ if status.is_server_error() => ProviderError::InternalError {
                    message: error.message.clone(),
                    provider: "openai".to_string(),
                    error_code: error.code.clone(),
                },
                _ => ProviderError::Other(anyhow::anyhow!("OpenAI API error: {}", error.message)),
            }
        } else {
            // Fallback error handling
            match status {
                StatusCode::TOO_MANY_REQUESTS => {
                    ProviderError::rate_limited_from_headers(&headers, None)
                }
                StatusCode::UNAUTHORIZED => ProviderError::AuthenticationFailed {
                    message: "Invalid API key".to_string(),
                    provider: "openai".to_string(),
                },
                _ => ProviderError::Other(anyhow::anyhow!("OpenAI API error: {}", status)),
            }
        }
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String, crate::Error> {
        let api_messages = self.convert_messages(messages);

        // GPT-5 only supports default temperature (1.0)
        let temperature = if self.config.model == "gpt-5" {
            None // Use default
        } else {
            Some(0.7)
        };

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            temperature,
            max_tokens: None,
            stream: Some(false),
            n: Some(1),
        };

        let url = self.build_url("chat/completions");

        let response = self
            .client
            .execute_with_retry(|client| {
                let config = self.config.clone();
                let url = url.clone();
                let request = request.clone();
                Box::pin(async move {
                    let mut req = client.post(&url).json(&request);

                    // Add Azure-specific header if needed
                    if config.is_azure {
                        req = req.header("api-key", &config.api_key);
                    }

                    // Add organization header if provided
                    if let Some(ref org_id) = config.organization_id {
                        req = req.header("OpenAI-Organization", org_id);
                    }

                    let response = req.send().await?;

                    if response.status().is_success() {
                        let completion: ChatCompletionResponse = response.json().await?;

                        completion
                            .choices
                            .first()
                            .map(|choice| choice.message.content.clone())
                            .ok_or_else(|| anyhow::anyhow!("No response from OpenAI"))
                    } else {
                        // Need to handle error here without self reference
                        let status = response.status();
                        let error_text = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "Failed to read error response".to_string());
                        Err(anyhow::anyhow!(
                            "OpenAI API error {}: {}",
                            status,
                            error_text
                        ))
                    }
                })
            })
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        Ok(response)
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<
        Box<dyn tokio_stream::Stream<Item = Result<StreamResponse, crate::Error>> + Send + Unpin>,
        crate::Error,
    > {
        let api_messages = self.convert_messages(messages);

        // GPT-5 only supports default temperature (1.0)
        let temperature = if self.config.model == "gpt-5" {
            None // Use default
        } else {
            Some(0.7)
        };

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            temperature,
            max_tokens: None,
            stream: Some(true),
            n: Some(1),
        };

        let url = self.build_url("chat/completions");
        let api_key = self.config.api_key.clone();
        let is_azure = self.config.is_azure;
        let org_id = self.config.organization_id.clone();

        let mut req = self.client.client.post(&url).json(&request);

        if is_azure {
            req = req.header("api-key", &api_key);
        } else {
            req = req.header("Authorization", format!("Bearer {api_key}"));
        }

        if let Some(ref org) = org_id {
            req = req.header("OpenAI-Organization", org);
        }

        let response = req
            .send()
            .await
            .map_err(|e| crate::Error::Provider(e.to_string()))?;

        if !response.status().is_success() {
            let error = self.handle_error_response(response).await;
            return Err(crate::Error::Provider(error.to_string()));
        }

        // Convert response body to stream of parsed events
        // Use bounded channel to prevent memory exhaustion under load
        let (tx, rx) = tokio::sync::mpsc::channel(1024);

        // Spawn task to process the response stream
        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut stream = response.bytes_stream();

            while let Some(chunk_result) = futures::StreamExt::next(&mut stream).await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        let lines: Vec<String> = buffer
                            .lines()
                            .map(std::string::ToString::to_string)
                            .collect();

                        for line in &lines {
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    if tx
                                        .send(Ok(StreamResponse {
                                            content: String::new(),
                                            done: true,
                                        }))
                                        .await
                                        .is_err()
                                    {
                                        break; // Receiver dropped
                                    }
                                } else if let Ok(chunk) = serde_json::from_str::<StreamChunk>(data)
                                    && let Some(choice) = chunk.choices.first()
                                    && let Some(content) = &choice.delta.content
                                    && tx
                                        .send(Ok(StreamResponse {
                                            content: content.clone(),
                                            done: choice.finish_reason.is_some(),
                                        }))
                                        .await
                                        .is_err()
                                {
                                    break; // Receiver dropped
                                }
                            }
                        }

                        // Clear processed lines from buffer
                        if let Some(last_newline) = buffer.rfind('\n') {
                            buffer.drain(..=last_newline);
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(crate::Error::Provider(e.to_string()))).await;
                        break;
                    }
                }
            }
        });

        Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn name(&self) -> &'static str {
        "openai"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_azure_url_building() {
        let config = OpenAIConfig {
            base_url: "https://myinstance.openai.azure.com".to_string(),
            model: "gpt-4".to_string(),
            is_azure: true,
            api_version: Some("2024-02-01".to_string()),
            ..Default::default()
        };

        let provider = OpenAIProvider::new(config).unwrap();
        let url = provider.build_url("chat/completions");

        assert_eq!(
            url,
            "https://myinstance.openai.azure.com/openai/deployments/gpt-4/chat/completions?api-version=2024-02-01"
        );
    }

    #[test]
    fn test_openai_url_building() {
        let config = OpenAIConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4".to_string(),
            is_azure: false,
            ..Default::default()
        };

        let provider = OpenAIProvider::new(config).unwrap();
        let url = provider.build_url("chat/completions");

        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn test_message_conversion() {
        let config = OpenAIConfig::default();
        let provider = OpenAIProvider::new(config).unwrap();

        let messages = vec![
            Message {
                role: Role::System,
                content: "You are a helpful assistant".to_string(),
                images: None,
            },
            Message {
                role: Role::User,
                content: "Hello".to_string(),
                images: None,
            },
        ];

        let api_messages = provider.convert_messages(messages);

        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].role, "system");
        assert_eq!(api_messages[1].role, "user");
    }
}
