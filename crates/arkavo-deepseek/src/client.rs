use crate::anthropic_compat::add_json_instruction_if_needed;
use crate::error::{DeepSeekError, Result};
use crate::stream::{SseStream, StreamResponse};
use crate::strict_mode::{validate_strict_schema, validate_tool_arguments};
use crate::types::{
    ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ErrorResponse, Model,
    ResponseFormat, Tool, ToolChoice,
};
use futures::Stream;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tracing::{debug, warn};

/// DeepSeek API configuration
#[derive(Clone, Debug)]
pub struct DeepSeekConfig {
    /// API key for authentication
    pub api_key: String,
    /// Base URL for API
    pub base_url: String,
    /// Model to use
    pub model: String,
    /// Use strict mode (beta features)
    pub use_strict_mode: bool,
    /// Use Anthropic compatibility mode
    pub anthropic_compat: bool,
    /// Enable thinking mode (required for V3.2-Speciale)
    pub thinking_mode: bool,
    /// Maximum tokens to generate
    pub max_tokens: Option<u32>,
    /// Temperature for generation
    pub temperature: Option<f32>,
    /// Top-p for generation
    pub top_p: Option<f32>,
    /// Request timeout
    pub timeout: Duration,
    /// Maximum retries
    pub max_retries: u32,
}

impl Default for DeepSeekConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.deepseek.com".to_string(),
            model: "deepseek-chat".to_string(),
            use_strict_mode: false,
            anthropic_compat: false,
            thinking_mode: false,
            max_tokens: Some(4096),
            temperature: Some(0.7),
            top_p: None,
            timeout: Duration::from_mins(1),
            max_retries: 3,
        }
    }
}

/// DeepSeek client for making API calls
pub struct DeepSeekClient {
    config: DeepSeekConfig,
    client: Client,
    /// Circuit breaker: marks endpoint as disabled after consecutive failures
    endpoint_disabled: Arc<AtomicBool>,
    /// Track consecutive failures for circuit breaker
    consecutive_failures: Arc<AtomicU32>,
}

impl DeepSeekClient {
    /// Maximum consecutive failures before circuit breaker opens
    const MAX_CONSECUTIVE_FAILURES: u32 = 5;

    /// Create a new DeepSeek client
    pub fn new(mut config: DeepSeekConfig) -> Result<Self> {
        // Validate base URL
        url::Url::parse(&config.base_url).map_err(|e| DeepSeekError::ConfigError {
            message: format!("Invalid base URL '{}': {}", config.base_url, e),
        })?;

        // Auto-enable thinking mode for V3.2-Speciale endpoint
        if config.base_url.contains("speciale") && !config.thinking_mode {
            debug!(
                "Auto-enabling thinking mode for V3.2-Speciale endpoint: {}",
                config.base_url
            );
            config.thinking_mode = true;
        }

        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| DeepSeekError::ConfigError {
                message: format!("Failed to create HTTP client: {e}"),
            })?;

        Ok(Self {
            config,
            client,
            endpoint_disabled: Arc::new(AtomicBool::new(false)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
        })
    }

    /// Mark endpoint as having a failure (for circuit breaker)
    fn mark_endpoint_failure(&self) {
        let failures = self.consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
        if failures >= Self::MAX_CONSECUTIVE_FAILURES {
            warn!(
                "DeepSeek endpoint {} marked as unusable after {} consecutive failures",
                self.config.base_url, failures
            );
            self.endpoint_disabled.store(true, Ordering::SeqCst);
        }
    }

    /// Mark endpoint as having succeeded (resets circuit breaker)
    fn mark_endpoint_success(&self) {
        self.consecutive_failures.store(0, Ordering::SeqCst);
    }

    /// Check if endpoint is usable (circuit breaker is closed)
    pub fn is_endpoint_usable(&self) -> bool {
        !self.endpoint_disabled.load(Ordering::SeqCst)
    }

    /// Check endpoint usability and return error if disabled
    fn check_endpoint(&self) -> Result<()> {
        if !self.is_endpoint_usable() {
            return Err(DeepSeekError::EndpointUnavailable {
                message: format!(
                    "DeepSeek endpoint {} disabled after {} consecutive failures. \
                     The endpoint may be experiencing issues. Try again later or use a different model.",
                    self.config.base_url,
                    Self::MAX_CONSECUTIVE_FAILURES
                ),
            });
        }
        Ok(())
    }

    /// Create from environment variables
    pub fn from_env() -> Result<Self> {
        let api_key =
            std::env::var("DEEPSEEK_API_KEY").map_err(|_| DeepSeekError::ConfigError {
                message: "DEEPSEEK_API_KEY environment variable not set".to_string(),
            })?;

        let base_url = std::env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com".to_string());

        let model = std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string());

        let use_strict_mode = std::env::var("DEEPSEEK_STRICT_MODE")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);

        let config = DeepSeekConfig {
            api_key,
            base_url,
            model,
            use_strict_mode,
            ..Default::default()
        };

        Self::new(config)
    }

    /// Set the model to use
    pub fn with_model(mut self, model: String) -> Self {
        self.config.model = model;
        self
    }

    /// Enable or disable strict mode
    pub fn with_strict_mode(mut self, enabled: bool) -> Self {
        self.config.use_strict_mode = enabled;
        if enabled {
            // Update base URL to beta endpoint
            if !self.config.base_url.contains("/beta") {
                self.config.base_url = "https://api.deepseek.com/beta".to_string();
            }
        }
        self
    }

    /// Get the appropriate endpoint URL
    fn get_endpoint(&self) -> String {
        if self.config.anthropic_compat {
            format!("{}/messages", self.config.base_url)
        } else {
            format!("{}/chat/completions", self.config.base_url)
        }
    }

    /// Prepare request with model fallback if needed
    fn prepare_request(
        &self,
        mut messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
        response_format: Option<ResponseFormat>,
    ) -> Result<ChatCompletionRequest> {
        // Check if model supports tools
        let model = Model::DeepSeekChat; // Default to chat for now
        let actual_model = if tools.is_some() && !model.supports_tools() {
            warn!(
                "Model {} does not support tools, falling back to deepseek-chat",
                self.config.model
            );
            "deepseek-chat".to_string()
        } else {
            self.config.model.clone()
        };

        // Validate and prepare tools for strict mode
        let validated_tools = if self.config.use_strict_mode {
            tools
                .map(|tools| {
                    tools
                        .into_iter()
                        .map(|mut tool| {
                            // Validate schema
                            validate_strict_schema(&tool.function.parameters)?;
                            // Set strict flag
                            tool.strict = Some(true);
                            Ok(tool)
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
        } else {
            tools
        };

        // Add JSON instruction if needed
        add_json_instruction_if_needed(&mut messages, &response_format);

        // Enable thinking mode if configured (required for V3.2-Speciale)
        let thinking = if self.config.thinking_mode {
            Some(crate::types::ThinkingConfig::enabled())
        } else {
            None
        };

        Ok(ChatCompletionRequest {
            model: actual_model,
            messages,
            tools: validated_tools,
            tool_choice,
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            max_tokens: self.config.max_tokens,
            stream: Some(false),
            stop: None,
            response_format,
            logprobs: None,
            n: None,
            thinking,
        })
    }

    /// Make a completion request
    pub async fn complete(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
        response_format: Option<ResponseFormat>,
    ) -> Result<ChatCompletionResponse> {
        // Check circuit breaker before making request
        self.check_endpoint()?;

        let request = self.prepare_request(messages, tools, tool_choice, response_format)?;

        let mut retries = 0;
        loop {
            // Check circuit breaker on each retry attempt (not just the first)
            self.check_endpoint()?;

            let response = self
                .client
                .post(self.get_endpoint())
                .bearer_auth(&self.config.api_key)
                .json(&request)
                .send()
                .await
                .map_err(|e| {
                    self.mark_endpoint_failure();
                    DeepSeekError::NetworkError {
                        message: e.to_string(),
                    }
                })?;

            if response.status().is_success() {
                self.mark_endpoint_success();
                return response
                    .json()
                    .await
                    .map_err(|e| DeepSeekError::StreamError {
                        message: format!("Failed to parse response: {e}"),
                    });
            }

            let error = self.handle_error_response(response).await?;

            // Mark failure for non-retryable errors or endpoint-specific errors
            if matches!(
                error,
                DeepSeekError::ModelNotFound { .. } | DeepSeekError::AuthenticationFailed { .. }
            ) {
                self.mark_endpoint_failure();
            }

            if error.is_retryable() && retries < self.config.max_retries {
                retries += 1;
                if let Some(duration) = error.retry_after() {
                    debug!("Rate limited, retrying after {:?}", duration);
                    tokio::time::sleep(duration).await;
                } else {
                    // Exponential backoff
                    let delay = Duration::from_millis(100 * 2_u64.pow(retries));
                    tokio::time::sleep(delay).await;
                }
            } else {
                return Err(error);
            }
        }
    }

    /// Make a streaming completion request
    pub async fn stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
        response_format: Option<ResponseFormat>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamResponse>> + Send>>> {
        // Check circuit breaker before making request
        self.check_endpoint()?;

        let mut request = self.prepare_request(messages, tools, tool_choice, response_format)?;
        request.stream = Some(true);

        let response = self
            .client
            .post(self.get_endpoint())
            .bearer_auth(&self.config.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                self.mark_endpoint_failure();
                DeepSeekError::NetworkError {
                    message: e.to_string(),
                }
            })?;

        if !response.status().is_success() {
            let error = self.handle_error_response(response).await?;
            // Mark failure for non-retryable endpoint errors
            if matches!(
                error,
                DeepSeekError::ModelNotFound { .. } | DeepSeekError::AuthenticationFailed { .. }
            ) {
                self.mark_endpoint_failure();
            }
            return Err(error);
        }

        self.mark_endpoint_success();
        let stream = SseStream::new(response.bytes_stream());
        Ok(Box::pin(stream))
    }

    /// Handle error responses
    async fn handle_error_response(&self, response: reqwest::Response) -> Result<DeepSeekError> {
        let status = response.status();
        let headers = response.headers().clone();

        // Try to parse error body
        if let Ok(error_response) = response.json::<ErrorResponse>().await {
            let error = &error_response.error;

            let deepseek_error = match status {
                StatusCode::TOO_MANY_REQUESTS => {
                    DeepSeekError::rate_limited_from_headers(&headers, Some(error.message.clone()))
                }
                StatusCode::UNAUTHORIZED => DeepSeekError::AuthenticationFailed {
                    message: error.message.clone(),
                },
                StatusCode::NOT_FOUND => {
                    if error.message.contains("model") {
                        DeepSeekError::ModelNotFound {
                            model: self.config.model.clone(),
                        }
                    } else {
                        DeepSeekError::InvalidRequest {
                            message: error.message.clone(),
                        }
                    }
                }
                StatusCode::BAD_REQUEST => {
                    // Check if it's a schema error
                    if error.message.contains("schema") || error.message.contains("strict") {
                        DeepSeekError::SchemaError {
                            message: error.message.clone(),
                            details: None,
                        }
                    } else {
                        DeepSeekError::InvalidRequest {
                            message: error.message.clone(),
                        }
                    }
                }
                _ => DeepSeekError::InternalError {
                    message: error.message.clone(),
                },
            };

            Ok(deepseek_error)
        } else {
            // Fallback error handling
            Ok(match status {
                StatusCode::TOO_MANY_REQUESTS => {
                    DeepSeekError::rate_limited_from_headers(&headers, None)
                }
                StatusCode::UNAUTHORIZED => DeepSeekError::AuthenticationFailed {
                    message: "Invalid API key".to_string(),
                },
                _ => DeepSeekError::Other(anyhow::anyhow!("DeepSeek API error: {status}")),
            })
        }
    }

    /// Validate tool arguments before execution
    pub fn validate_tool_arguments(
        &self,
        arguments: &str,
        schema: &Value,
        tool_name: &str,
    ) -> Result<Value> {
        validate_tool_arguments(arguments, schema, tool_name)
    }

    /// Get the current configuration (for testing)
    #[cfg(test)]
    pub fn config(&self) -> &DeepSeekConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_enable_thinking_mode_for_speciale_url() {
        let config = DeepSeekConfig {
            api_key: "test-key".to_string(),
            base_url: "https://api.deepseek.com/v3.2_speciale_expires_on_20251215".to_string(),
            thinking_mode: false, // Explicitly set to false
            ..Default::default()
        };

        let client = DeepSeekClient::new(config).unwrap();
        assert!(
            client.config().thinking_mode,
            "thinking_mode should be auto-enabled for speciale URL"
        );
    }

    #[test]
    fn test_standard_url_does_not_auto_enable_thinking() {
        let config = DeepSeekConfig {
            api_key: "test-key".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            thinking_mode: false,
            ..Default::default()
        };

        let client = DeepSeekClient::new(config).unwrap();
        assert!(
            !client.config().thinking_mode,
            "thinking_mode should remain false for standard URL"
        );
    }

    #[test]
    fn test_thinking_mode_preserved_if_already_true() {
        let config = DeepSeekConfig {
            api_key: "test-key".to_string(),
            base_url: "https://api.deepseek.com/v3.2_speciale_expires_on_20251215".to_string(),
            thinking_mode: true, // Already enabled
            ..Default::default()
        };

        let client = DeepSeekClient::new(config).unwrap();
        assert!(client.config().thinking_mode);
    }

    #[test]
    fn test_invalid_base_url_returns_error() {
        let config = DeepSeekConfig {
            api_key: "test-key".to_string(),
            base_url: "not a valid url".to_string(),
            ..Default::default()
        };

        let result = DeepSeekClient::new(config);
        assert!(result.is_err());
        match result {
            Err(DeepSeekError::ConfigError { message }) => {
                assert!(message.contains("Invalid base URL"));
            }
            _ => panic!("Expected ConfigError"),
        }
    }

    #[test]
    fn test_circuit_breaker_initially_open() {
        let config = DeepSeekConfig {
            api_key: "test-key".to_string(),
            ..Default::default()
        };

        let client = DeepSeekClient::new(config).unwrap();
        assert!(
            client.is_endpoint_usable(),
            "Circuit breaker should be closed initially"
        );
    }
}
