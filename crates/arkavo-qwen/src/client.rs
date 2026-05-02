use crate::error::{QwenError, Result};
use crate::models::QwenModel;
use crate::stream::SseStream;
use crate::types::{
    ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ErrorResponse, Tool, ToolChoice,
};
use reqwest::{Client, StatusCode};
use std::time::Duration;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum QwenRegion {
    #[default]
    International,
    China,
}

impl QwenRegion {
    pub fn base_url(&self) -> &'static str {
        match self {
            QwenRegion::International => "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
            QwenRegion::China => "https://dashscope.aliyuncs.com/compatible-mode/v1",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "intl" | "international" => Some(QwenRegion::International),
            "cn" | "china" => Some(QwenRegion::China),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct QwenConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub region: QwenRegion,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub timeout: Duration,
    pub max_retries: u32,
}

impl Default for QwenConfig {
    fn default() -> Self {
        let region = QwenRegion::default();
        Self {
            api_key: String::new(),
            base_url: region.base_url().to_string(),
            model: QwenModel::default().as_str().to_string(),
            region,
            max_tokens: Some(4096),
            temperature: Some(0.7),
            top_p: None,
            timeout: Duration::from_mins(1),
            max_retries: 3,
        }
    }
}

pub struct QwenClient {
    config: QwenConfig,
    client: Client,
}

impl QwenClient {
    pub fn new(config: QwenConfig) -> Result<Self> {
        url::Url::parse(&config.base_url).map_err(|e| QwenError::ConfigError {
            message: format!("Invalid base URL '{}': {}", config.base_url, e),
        })?;

        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| QwenError::ConfigError {
                message: format!("Failed to create HTTP client: {e}"),
            })?;

        Ok(Self { config, client })
    }

    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("DASHSCOPE_API_KEY").map_err(|_| QwenError::ConfigError {
            message: "DASHSCOPE_API_KEY environment variable not set".to_string(),
        })?;

        let region = std::env::var("DASHSCOPE_REGION")
            .ok()
            .and_then(|r| QwenRegion::parse(&r))
            .unwrap_or_default();

        let base_url = match std::env::var("DASHSCOPE_BASE_URL") {
            Ok(url) => url,
            Err(_) => region.base_url().to_string(),
        };

        let model = match std::env::var("QWEN_MODEL") {
            Ok(m) => m,
            Err(_) => QwenModel::default().as_str().to_string(),
        };

        let config = QwenConfig {
            api_key,
            base_url,
            model,
            region,
            ..Default::default()
        };

        Self::new(config)
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.config.temperature = Some(temperature);
        self
    }

    pub fn with_top_p(mut self, top_p: f32) -> Self {
        self.config.top_p = Some(top_p);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.config.max_tokens = Some(max_tokens);
        self
    }

    pub async fn complete(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
    ) -> Result<ChatCompletionResponse> {
        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            tools,
            tool_choice,
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            max_tokens: self.config.max_tokens,
            stream: Some(false),
            stop: None,
        };

        self.send_request(request).await
    }

    pub async fn stream(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
        tool_choice: Option<ToolChoice>,
    ) -> Result<SseStream> {
        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            tools,
            tool_choice,
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            max_tokens: self.config.max_tokens,
            stream: Some(true),
            stop: None,
        };

        self.send_stream_request(request).await
    }

    async fn send_request(&self, request: ChatCompletionRequest) -> Result<ChatCompletionResponse> {
        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                let backoff = self.calculate_backoff(attempt, last_error.as_ref());
                debug!(
                    "Retry attempt {}/{}, waiting {}ms",
                    attempt,
                    self.config.max_retries,
                    backoff.as_millis()
                );
                tokio::time::sleep(backoff).await;
            }

            match self.try_send_request(&request).await {
                Ok(response) => return Ok(response),
                Err(e) if e.is_retryable() && attempt < self.config.max_retries => {
                    debug!("Request failed with retryable error: {}", e);
                    last_error = Some(e);
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_error.unwrap_or_else(|| QwenError::ApiError {
            status: 500,
            message: "Max retries exceeded".to_string(),
        }))
    }

    async fn try_send_request(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse> {
        let url = format!("{}/chat/completions", self.config.base_url);

        debug!("Sending request to Qwen API: {}", url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(request)
            .send()
            .await?;

        let status = response.status();
        let headers = response.headers().clone();

        if !status.is_success() {
            let error_text = response.text().await?;

            if let Ok(error_response) = serde_json::from_str::<ErrorResponse>(&error_text) {
                return Err(self.map_api_error(status, error_response, &headers));
            }

            return Err(QwenError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let completion: ChatCompletionResponse = response.json().await?;
        Ok(completion)
    }

    fn calculate_backoff(&self, attempt: u32, error: Option<&QwenError>) -> Duration {
        if let Some(QwenError::RateLimitExceeded {
            retry_after: Some(seconds),
            ..
        }) = error
        {
            return Duration::from_secs(*seconds);
        }

        let base_ms = 1000u64;
        let backoff_ms = base_ms * 2u64.pow(attempt - 1);
        let max_backoff_ms = 60_000u64;

        Duration::from_millis(backoff_ms.min(max_backoff_ms))
    }

    async fn send_stream_request(&self, request: ChatCompletionRequest) -> Result<SseStream> {
        let url = format!("{}/chat/completions", self.config.base_url);

        debug!("Sending streaming request to Qwen API: {}", url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        let headers = response.headers().clone();

        if !status.is_success() {
            let error_text = response.text().await?;

            if let Ok(error_response) = serde_json::from_str::<ErrorResponse>(&error_text) {
                return Err(self.map_api_error(status, error_response, &headers));
            }

            return Err(QwenError::ApiError {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let stream = response.bytes_stream();
        Ok(SseStream::new(stream))
    }

    fn map_api_error(
        &self,
        status: StatusCode,
        error: ErrorResponse,
        headers: &reqwest::header::HeaderMap,
    ) -> QwenError {
        match status {
            StatusCode::UNAUTHORIZED => QwenError::AuthenticationFailed {
                message: error.error.message,
            },
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = headers
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());

                QwenError::RateLimitExceeded {
                    message: error.error.message,
                    retry_after,
                }
            }
            StatusCode::NOT_FOUND => {
                if error.error.message.to_lowercase().contains("model") {
                    QwenError::ModelNotFound {
                        model: self.config.model.clone(),
                    }
                } else {
                    QwenError::ApiError {
                        status: status.as_u16(),
                        message: error.error.message,
                    }
                }
            }
            StatusCode::BAD_REQUEST => QwenError::InvalidRequest {
                message: error.error.message,
            },
            _ => QwenError::ApiError {
                status: status.as_u16(),
                message: error.error.message,
            },
        }
    }

    pub fn config(&self) -> &QwenConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_base_urls() {
        assert_eq!(
            QwenRegion::International.base_url(),
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(
            QwenRegion::China.base_url(),
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
    }

    #[test]
    fn test_region_parse() {
        assert_eq!(QwenRegion::parse("intl"), Some(QwenRegion::International));
        assert_eq!(QwenRegion::parse("cn"), Some(QwenRegion::China));
        assert_eq!(QwenRegion::parse("invalid"), None);
    }

    #[test]
    fn test_config_builder() {
        let config = QwenConfig::default();
        let client = QwenClient::new(config).unwrap();

        let client = client
            .with_model("qwen3-14b")
            .with_temperature(0.8)
            .with_top_p(0.9)
            .with_max_tokens(2048);

        assert_eq!(client.config.model, "qwen3-14b");
        assert_eq!(client.config.temperature, Some(0.8));
        assert_eq!(client.config.top_p, Some(0.9));
        assert_eq!(client.config.max_tokens, Some(2048));
    }

    #[test]
    fn test_default_config() {
        let config = QwenConfig::default();
        assert_eq!(config.region, QwenRegion::International);
        assert_eq!(
            config.base_url,
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(config.model, "qwen3-32b");
        assert_eq!(config.max_tokens, Some(4096));
        assert_eq!(config.temperature, Some(0.7));
    }
}
