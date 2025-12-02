use std::time::Duration;
use thiserror::Error;

/// DeepSeek provider error types
#[derive(Error, Debug)]
pub enum DeepSeekError {
    /// Rate limit exceeded
    #[error("Rate limit exceeded{}", .retry_after.map(|d| format!(" (retry after {d:?})")).unwrap_or_default())]
    RateLimited {
        retry_after: Option<Duration>,
        message: Option<String>,
    },

    /// Authentication failed
    #[error("Authentication failed: {message}")]
    AuthenticationFailed { message: String },

    /// Model not found or not available
    #[error("Model '{model}' not found")]
    ModelNotFound { model: String },

    /// Invalid request
    #[error("Invalid request: {message}")]
    InvalidRequest { message: String },

    /// Schema validation error (strict mode)
    #[error("Schema validation failed: {message}")]
    SchemaError {
        message: String,
        details: Option<serde_json::Value>,
    },

    /// Unsupported message part (Anthropic compatibility)
    #[error("Unsupported message part: {message}")]
    UnsupportedMessagePart { message: String, part_type: String },

    /// Tool arguments invalid
    #[error("Tool arguments invalid: {message}")]
    ToolArgumentsInvalid {
        message: String,
        tool_name: String,
        arguments: String,
    },

    /// Network error
    #[error("Network error: {message}")]
    NetworkError { message: String },

    /// Timeout
    #[error("Request timed out after {duration:?}")]
    Timeout { duration: Duration },

    /// Internal error
    #[error("Internal error: {message}")]
    InternalError { message: String },

    /// Configuration error
    #[error("Configuration error: {message}")]
    ConfigError { message: String },

    /// Streaming error
    #[error("Stream error: {message}")]
    StreamError { message: String },

    /// Endpoint unavailable (circuit breaker open)
    #[error("Endpoint unavailable: {message}")]
    EndpointUnavailable { message: String },

    /// Generic error wrapper
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl DeepSeekError {
    /// Check if the error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            DeepSeekError::RateLimited { .. }
                | DeepSeekError::NetworkError { .. }
                | DeepSeekError::Timeout { .. }
        )
    }

    /// Get retry after duration if available
    pub fn retry_after(&self) -> Option<Duration> {
        if let DeepSeekError::RateLimited { retry_after, .. } = self {
            *retry_after
        } else {
            None
        }
    }

    /// Create rate limited error from headers
    pub fn rate_limited_from_headers(
        headers: &reqwest::header::HeaderMap,
        message: Option<String>,
    ) -> Self {
        let retry_after = headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs);

        DeepSeekError::RateLimited {
            retry_after,
            message,
        }
    }
}

pub type Result<T> = std::result::Result<T, DeepSeekError>;
