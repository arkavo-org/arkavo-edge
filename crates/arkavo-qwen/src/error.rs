use thiserror::Error;

pub type Result<T> = std::result::Result<T, QwenError>;

#[derive(Error, Debug)]
pub enum QwenError {
    #[error("Configuration error: {message}")]
    ConfigError { message: String },

    #[error("Authentication failed: {message}")]
    AuthenticationFailed { message: String },

    #[error("Rate limit exceeded: {message}, retry_after: {retry_after:?}")]
    RateLimitExceeded {
        message: String,
        retry_after: Option<u64>,
    },

    #[error("Model not found: {model}")]
    ModelNotFound { model: String },

    #[error("Invalid request: {message}")]
    InvalidRequest { message: String },

    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Stream error: {message}")]
    StreamError { message: String },

    #[error("Vision not supported: {model}")]
    VisionNotSupported { model: String },

    #[error("Tools not supported: {model}")]
    ToolsNotSupported { model: String },

    #[error("Internal error: {message}")]
    InternalError { message: String },

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl QwenError {
    pub fn is_retryable(&self) -> bool {
        match self {
            QwenError::RateLimitExceeded { .. } => true,
            QwenError::ApiError { status, .. } if *status >= 500 => true,
            QwenError::NetworkError(_) => true,
            _ => false,
        }
    }

    pub fn retry_after(&self) -> Option<u64> {
        match self {
            QwenError::RateLimitExceeded { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = QwenError::ConfigError {
            message: "Invalid API key".to_string(),
        };
        assert_eq!(err.to_string(), "Configuration error: Invalid API key");

        let err = QwenError::ModelNotFound {
            model: "qwen3-999b".to_string(),
        };
        assert_eq!(err.to_string(), "Model not found: qwen3-999b");
    }

    #[test]
    fn test_retryable_errors() {
        assert!(
            QwenError::RateLimitExceeded {
                message: "Too many requests".to_string(),
                retry_after: Some(60),
            }
            .is_retryable()
        );

        assert!(
            QwenError::ApiError {
                status: 503,
                message: "Service unavailable".to_string(),
            }
            .is_retryable()
        );

        assert!(
            !QwenError::ConfigError {
                message: "Bad config".to_string(),
            }
            .is_retryable()
        );

        assert!(
            !QwenError::ApiError {
                status: 400,
                message: "Bad request".to_string(),
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_retry_after() {
        let err = QwenError::RateLimitExceeded {
            message: "Rate limited".to_string(),
            retry_after: Some(120),
        };
        assert_eq!(err.retry_after(), Some(120));

        let err = QwenError::ConfigError {
            message: "Bad config".to_string(),
        };
        assert_eq!(err.retry_after(), None);
    }
}
