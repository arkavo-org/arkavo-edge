//! Error types for arkavo-security

use thiserror::Error;

/// Security-related errors
#[derive(Error, Debug)]
pub enum SecurityError {
    /// Authentication error
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// TLS/SSL error
    #[error("TLS error: {0}")]
    Tls(String),

    /// Validation error
    #[error("Validation error: {0}")]
    Validation(String),

    /// Rate limit exceeded
    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    /// Configuration error
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JWT error
    #[error("JWT error: {0}")]
    Jwt(String),

    /// OAuth2 error
    #[error("OAuth2 error: {0}")]
    OAuth2(String),

    /// Token error
    #[error("Token error: {0}")]
    Token(String),

    /// Invalid request
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type alias for security operations
pub type Result<T> = std::result::Result<T, SecurityError>;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SecurityError::Auth("Invalid token".to_string());
        assert_eq!(err.to_string(), "Authentication failed: Invalid token");

        let err = SecurityError::Tls("Certificate expired".to_string());
        assert_eq!(err.to_string(), "TLS error: Certificate expired");

        let err = SecurityError::RateLimitExceeded;
        assert_eq!(err.to_string(), "Rate limit exceeded");
    }

    #[test]
    fn test_error_from_serde() {
        let json = "{ invalid json }";
        let result: Result<serde_json::Value> =
            serde_json::from_str(json).map_err(SecurityError::from);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SecurityError::Serialization(_)
        ));
    }
}
