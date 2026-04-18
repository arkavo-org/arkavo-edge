use std::time::Duration;
use thiserror::Error;

/// Comprehensive error types for LLM providers
#[derive(Error, Debug)]
pub enum ProviderError {
    /// Rate limit exceeded with optional retry-after duration
    #[error("Rate limit exceeded{}", .retry_after.map(|d| format!(" (retry after {d:?})")).unwrap_or_default())]
    RateLimited {
        retry_after: Option<Duration>,
        message: Option<String>,
    },

    /// Authentication failed
    #[error("Authentication failed: {message}")]
    AuthenticationFailed { message: String, provider: String },

    /// Model not found or not available
    #[error("Model '{model}' not found on provider '{provider}'")]
    ModelNotFound {
        model: String,
        provider: String,
        available_models: Option<Vec<String>>,
    },

    /// Invalid request (e.g., malformed JSON, invalid parameters)
    #[error("Invalid request: {message}")]
    InvalidRequest {
        message: String,
        details: Option<serde_json::Value>,
    },

    /// Network error (connection failed, DNS resolution, etc.)
    #[error("Network error: {message}")]
    NetworkError {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Request timeout
    #[error("Request timed out after {duration:?}")]
    Timeout {
        duration: Duration,
        operation: String,
    },

    /// Circuit breaker is open
    #[error("Circuit breaker is open for provider '{provider}' (failures: {consecutive_failures})")]
    CircuitOpen {
        provider: String,
        consecutive_failures: u32,
        last_failure: chrono::DateTime<chrono::Utc>,
    },

    /// Provider is unavailable or unhealthy
    #[error("Provider '{provider}' is unavailable: {reason}")]
    ProviderUnavailable { provider: String, reason: String },

    /// Configuration error
    #[error("Configuration error: {message}")]
    ConfigurationError {
        message: String,
        field: Option<String>,
    },

    /// Token limit exceeded
    #[error("Token limit exceeded: used {used} tokens, limit is {limit}")]
    TokenLimitExceeded {
        used: usize,
        limit: usize,
        model: String,
    },

    /// Content filtered or blocked
    #[error("Content was filtered or blocked: {reason}")]
    ContentFiltered {
        reason: String,
        category: Option<String>,
    },

    /// Internal provider error
    #[error("Internal provider error: {message}")]
    InternalError {
        message: String,
        provider: String,
        error_code: Option<String>,
    },

    /// Generic error wrapper
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl ProviderError {
    /// Check if the error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimited { .. }
                | ProviderError::NetworkError { .. }
                | ProviderError::Timeout { .. }
                | ProviderError::ProviderUnavailable { .. }
                | ProviderError::InternalError { .. }
        )
    }

    /// Get retry delay if applicable
    pub fn retry_delay(&self) -> Option<Duration> {
        match self {
            ProviderError::RateLimited { retry_after, .. } => *retry_after,
            ProviderError::NetworkError { .. } => Some(Duration::from_secs(1)),
            ProviderError::Timeout { .. } => Some(Duration::from_secs(2)),
            ProviderError::InternalError { .. } => Some(Duration::from_secs(5)),
            _ => None,
        }
    }

    /// Check if this is an authentication error
    pub fn is_auth_error(&self) -> bool {
        matches!(self, ProviderError::AuthenticationFailed { .. })
    }

    /// Check if this is a client error (4xx class)
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            ProviderError::InvalidRequest { .. }
                | ProviderError::ModelNotFound { .. }
                | ProviderError::TokenLimitExceeded { .. }
                | ProviderError::ContentFiltered { .. }
        )
    }

    /// Check if this is a server error (5xx class)
    pub fn is_server_error(&self) -> bool {
        matches!(
            self,
            ProviderError::InternalError { .. } | ProviderError::ProviderUnavailable { .. }
        )
    }
}

/// Helper functions for creating specific errors
impl ProviderError {
    /// Create a rate limited error from HTTP response
    pub fn rate_limited_from_headers(
        headers: &reqwest::header::HeaderMap,
        message: Option<String>,
    ) -> Self {
        let retry_after = headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs);

        Self::RateLimited {
            retry_after,
            message,
        }
    }

    /// Create a network error from a reqwest error
    pub fn from_reqwest_error(err: reqwest::Error) -> Self {
        let message = if err.is_connect() {
            "Connection failed".to_string()
        } else if err.is_timeout() {
            return Self::Timeout {
                duration: Duration::from_secs(30), // Default, should be overridden
                operation: "HTTP request".to_string(),
            };
        } else if err.is_decode() {
            "Failed to decode response".to_string()
        } else {
            err.to_string()
        };

        Self::NetworkError {
            message,
            source: Some(Box::new(err)),
        }
    }
}

/// Result type alias for provider operations
pub type ProviderResult<T> = Result<T, ProviderError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_categorization() {
        let rate_limit = ProviderError::RateLimited {
            retry_after: Some(Duration::from_mins(1)),
            message: Some("Too many requests".to_string()),
        };
        assert!(rate_limit.is_retryable());
        assert!(!rate_limit.is_auth_error());
        assert!(!rate_limit.is_client_error());

        let auth_error = ProviderError::AuthenticationFailed {
            message: "Invalid API key".to_string(),
            provider: "openai".to_string(),
        };
        assert!(!auth_error.is_retryable());
        assert!(auth_error.is_auth_error());

        let model_error = ProviderError::ModelNotFound {
            model: "gpt-5".to_string(),
            provider: "openai".to_string(),
            available_models: None,
        };
        assert!(!model_error.is_retryable());
        assert!(model_error.is_client_error());

        let server_error = ProviderError::InternalError {
            message: "Internal server error".to_string(),
            provider: "anthropic".to_string(),
            error_code: Some("500".to_string()),
        };
        assert!(server_error.is_retryable());
        assert!(server_error.is_server_error());
    }

    #[test]
    fn test_retry_delays() {
        let rate_limit = ProviderError::RateLimited {
            retry_after: Some(Duration::from_mins(2)),
            message: None,
        };
        assert_eq!(rate_limit.retry_delay(), Some(Duration::from_mins(2)));

        let network_error = ProviderError::NetworkError {
            message: "Connection reset".to_string(),
            source: None,
        };
        assert_eq!(network_error.retry_delay(), Some(Duration::from_secs(1)));

        let auth_error = ProviderError::AuthenticationFailed {
            message: "Invalid token".to_string(),
            provider: "gemini".to_string(),
        };
        assert_eq!(auth_error.retry_delay(), None);
    }
}
