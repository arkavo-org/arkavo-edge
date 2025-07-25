use reqwest::StatusCode;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum KimiError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("JSON parsing failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Authentication failed: {message}")]
    AuthenticationFailed { message: String },

    #[error("Model not found: {model}")]
    ModelNotFound { model: String },

    #[error("Invalid request: {message}")]
    InvalidRequest { message: String },

    #[error("Rate limit exceeded. Retry after {retry_after:?}")]
    RateLimitExceeded {
        retry_after: Option<Duration>,
        message: Option<String>,
    },

    #[error("Internal server error: {message}")]
    InternalError {
        message: String,
        error_code: Option<String>,
    },

    #[error("Stream error: {0}")]
    Stream(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

impl KimiError {
    /// Create rate limit error from response headers
    pub fn rate_limited_from_headers(
        headers: &reqwest::header::HeaderMap,
        message: Option<String>,
    ) -> Self {
        let retry_after = headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs);

        Self::RateLimitExceeded {
            retry_after,
            message,
        }
    }

    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimitExceeded { .. }
                | Self::InternalError { .. }
                | Self::Request(_)
                | Self::Stream(_)
        )
    }

    /// Get retry delay if applicable
    pub fn retry_delay(&self) -> Option<Duration> {
        match self {
            Self::RateLimitExceeded { retry_after, .. } => *retry_after,
            Self::InternalError { .. } => Some(Duration::from_secs(5)),
            Self::Request(_) | Self::Stream(_) => Some(Duration::from_secs(1)),
            _ => None,
        }
    }

    /// Create error from HTTP status code
    pub fn from_status(status: StatusCode, message: String) -> Self {
        match status {
            StatusCode::UNAUTHORIZED => Self::AuthenticationFailed { message },
            StatusCode::TOO_MANY_REQUESTS => Self::RateLimitExceeded {
                retry_after: None,
                message: Some(message),
            },
            StatusCode::BAD_REQUEST => Self::InvalidRequest { message },
            StatusCode::NOT_FOUND => {
                if message.contains("model") {
                    Self::ModelNotFound {
                        model: "unknown".to_string(),
                    }
                } else {
                    Self::InvalidRequest { message }
                }
            }
            _ if status.is_server_error() => Self::InternalError {
                message,
                error_code: None,
            },
            _ => Self::Other(anyhow::anyhow!("HTTP error {}: {}", status, message)),
        }
    }
}

pub type Result<T> = std::result::Result<T, KimiError>;
