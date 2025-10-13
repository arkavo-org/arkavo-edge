use crate::common::{HttpClientBuilder, HttpClientConfig, RetryableHttpClient};
use crate::common::{ProviderError, ProviderResult};
use crate::{Message, Provider, Role, StreamResponse};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Anthropic API configuration
#[derive(Clone, Debug)]
pub struct AnthropicConfig {
    /// API key for authentication
    pub api_key: String,
    /// Base URL for API
    pub base_url: String,
    /// Model to use (e.g., "claude-3-opus-20240229")
    pub model: String,
    /// API version
    pub api_version: String,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.anthropic.com".to_string(),
            model: "claude-3-opus-20240229".to_string(),
            api_version: "2023-06-01".to_string(),
        }
    }
}

/// Anthropic API request structures
#[derive(Debug, Clone, Serialize)]
struct CreateMessageRequest {
    model: String,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageResponse {
    id: String,
    content: Vec<ContentBlock>,
    model: String,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Usage {
    input_tokens: u32,
    output_tokens: u32,
}

/// Streaming response structures
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum StreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartData },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: ContentBlockData,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: DeltaData },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaData,
        usage: Usage,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "error")]
    Error { error: ErrorData },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageStartData {
    id: String,
    model: String,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ContentBlockData {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DeltaData {
    #[serde(rename = "type")]
    delta_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageDeltaData {
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ErrorData {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}

/// Error response structure
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ErrorResponse {
    #[serde(rename = "type")]
    error_type: String,
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    error_subtype: String,
    message: String,
}

/// Anthropic provider implementation
pub struct AnthropicProvider {
    config: AnthropicConfig,
    client: Arc<RetryableHttpClient>,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> ProviderResult<Self> {
        // Validate base URL
        url::Url::parse(&config.base_url)
            .map_err(|e| anyhow::anyhow!("Invalid base URL '{}': {}", config.base_url, e))?;

        let http_config = HttpClientConfig {
            base_url: config.base_url.clone(),
            auth_token: None, // Anthropic uses a custom header
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

    /// Convert messages to Anthropic's 3-role format
    fn convert_messages(&self, messages: Vec<Message>) -> (Option<String>, Vec<ApiMessage>) {
        let mut system_content = None;
        let mut api_messages = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    // Anthropic handles system messages separately
                    if system_content.is_none() {
                        system_content = Some(msg.content);
                    } else {
                        // If multiple system messages, concatenate them
                        system_content =
                            Some(format!("{}\n\n{}", system_content.unwrap(), msg.content));
                    }
                }
                Role::User => {
                    api_messages.push(ApiMessage {
                        role: "user".to_string(),
                        content: msg.content,
                    });
                }
                Role::Assistant => {
                    api_messages.push(ApiMessage {
                        role: "assistant".to_string(),
                        content: msg.content,
                    });
                }
            }
        }

        // Ensure conversation starts with user message
        if api_messages.is_empty() || api_messages[0].role != "user" {
            api_messages.insert(
                0,
                ApiMessage {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                },
            );
        }

        // Ensure alternating user/assistant messages
        let mut cleaned_messages: Vec<ApiMessage> = Vec::new();
        let mut last_role = None;

        for msg in api_messages {
            if last_role.as_ref() == Some(&msg.role) {
                // Same role as previous, merge content
                if let Some(last_msg) = cleaned_messages.last_mut() {
                    last_msg.content = format!("{}\n\n{}", last_msg.content, msg.content);
                }
            } else {
                cleaned_messages.push(msg.clone());
                last_role = Some(msg.role);
            }
        }

        (system_content, cleaned_messages)
    }

    /// Handle API errors
    async fn handle_error_response(&self, response: reqwest::Response) -> ProviderError {
        let status = response.status();
        let headers = response.headers().clone();

        // Try to parse error body
        if let Ok(error_response) = response.json::<ErrorResponse>().await {
            let error = &error_response.error;

            match error.error_subtype.as_str() {
                "rate_limit_error" => {
                    ProviderError::rate_limited_from_headers(&headers, Some(error.message.clone()))
                }
                "authentication_error" => ProviderError::AuthenticationFailed {
                    message: error.message.clone(),
                    provider: "anthropic".to_string(),
                },
                "not_found_error" => {
                    if error.message.contains("model") {
                        ProviderError::ModelNotFound {
                            model: self.config.model.clone(),
                            provider: "anthropic".to_string(),
                            available_models: None,
                        }
                    } else {
                        ProviderError::InvalidRequest {
                            message: error.message.clone(),
                            details: None,
                        }
                    }
                }
                "invalid_request_error" => ProviderError::InvalidRequest {
                    message: error.message.clone(),
                    details: None,
                },
                _ => ProviderError::InternalError {
                    message: error.message.clone(),
                    provider: "anthropic".to_string(),
                    error_code: Some(error.error_subtype.clone()),
                },
            }
        } else {
            // Fallback error handling
            match status {
                StatusCode::TOO_MANY_REQUESTS => {
                    ProviderError::rate_limited_from_headers(&headers, None)
                }
                StatusCode::UNAUTHORIZED => ProviderError::AuthenticationFailed {
                    message: "Invalid API key".to_string(),
                    provider: "anthropic".to_string(),
                },
                _ => ProviderError::Other(anyhow::anyhow!("Anthropic API error: {status}")),
            }
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String, crate::Error> {
        let (system_content, api_messages) = self.convert_messages(messages);

        let request = CreateMessageRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            max_tokens: Some(4096),
            temperature: Some(0.7),
            system: system_content,
            stream: Some(false),
        };

        let url = format!("{}/v1/messages", self.config.base_url);

        let response = self
            .client
            .execute_with_retry(|client| {
                let config = self.config.clone();
                let url = url.clone();
                let request = request.clone();
                Box::pin(async move {
                    let response = client
                        .post(&url)
                        .header("x-api-key", &config.api_key)
                        .header("anthropic-version", &config.api_version)
                        .header("content-type", "application/json")
                        .json(&request)
                        .send()
                        .await?;

                    if response.status().is_success() {
                        let message: MessageResponse = response.json().await?;

                        let content = message
                            .content
                            .iter()
                            .filter_map(|block| block.text.clone())
                            .collect::<Vec<_>>()
                            .join("\n");

                        Ok(content)
                    } else {
                        // Need to handle error here without self reference
                        let status = response.status();
                        let error_text = response
                            .text()
                            .await
                            .unwrap_or_else(|_| "Failed to read error response".to_string());
                        Err(anyhow::anyhow!(
                            "Anthropic API error {status}: {error_text}"
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
        let (system_content, api_messages) = self.convert_messages(messages);

        let request = CreateMessageRequest {
            model: self.config.model.clone(),
            messages: api_messages,
            max_tokens: Some(4096),
            temperature: Some(0.7),
            system: system_content,
            stream: Some(true),
        };

        let url = format!("{}/v1/messages", self.config.base_url);

        let response = self
            .client
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", &self.config.api_version)
            .header("content-type", "application/json")
            .json(&request)
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
                            if let Some(data) = line.strip_prefix("data: ")
                                && let Ok(event) = serde_json::from_str::<StreamEvent>(data)
                            {
                                match event {
                                    StreamEvent::ContentBlockDelta { delta, .. } => {
                                        if let Some(text) = delta.text
                                            && tx
                                                .send(Ok(StreamResponse {
                                                    content: text,
                                                    done: false,
                                                }))
                                                .await
                                                .is_err()
                                        {
                                            break; // Receiver dropped
                                        }
                                    }
                                    StreamEvent::MessageStop => {
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
                                    }
                                    StreamEvent::Error { error } => {
                                        let _ = tx
                                            .send(Err(crate::Error::Provider(format!(
                                                "Stream error: {}",
                                                error.message
                                            ))))
                                            .await;
                                        break;
                                    }
                                    _ => {} // Ignore other event types
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
        "anthropic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_conversion() {
        let config = AnthropicConfig::default();
        let provider = AnthropicProvider::new(config).unwrap();

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
            Message {
                role: Role::Assistant,
                content: "Hi there!".to_string(),
                images: None,
            },
            Message {
                role: Role::User,
                content: "How are you?".to_string(),
                images: None,
            },
        ];

        let (system, api_messages) = provider.convert_messages(messages);

        assert_eq!(system, Some("You are a helpful assistant".to_string()));
        assert_eq!(api_messages.len(), 3);
        assert_eq!(api_messages[0].role, "user");
        assert_eq!(api_messages[1].role, "assistant");
        assert_eq!(api_messages[2].role, "user");
    }

    #[test]
    fn test_message_deduplication() {
        let config = AnthropicConfig::default();
        let provider = AnthropicProvider::new(config).unwrap();

        let messages = vec![
            Message {
                role: Role::User,
                content: "First message".to_string(),
                images: None,
            },
            Message {
                role: Role::User,
                content: "Second message".to_string(),
                images: None,
            },
            Message {
                role: Role::Assistant,
                content: "Response".to_string(),
                images: None,
            },
        ];

        let (_, api_messages) = provider.convert_messages(messages);

        assert_eq!(api_messages.len(), 2);
        assert_eq!(api_messages[0].content, "First message\n\nSecond message");
        assert_eq!(api_messages[1].role, "assistant");
    }
}
