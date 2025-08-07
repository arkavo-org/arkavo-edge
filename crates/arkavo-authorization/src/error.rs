use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthorizationError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    ConfigError(String),

    #[error("Authorization denied")]
    Denied,

    #[error("Entity resolution failed: {0}")]
    EntityResolutionError(String),

    #[error("Invalid JWT token: {0}")]
    InvalidToken(String),

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("Timeout waiting for response")]
    Timeout,

    #[error("Service unavailable")]
    ServiceUnavailable,

    #[error("Invalid response from server: {0}")]
    InvalidResponse(String),
}

pub type Result<T> = std::result::Result<T, AuthorizationError>;
