//! Error types for session management
//!
//! This module provides error types for the session crate.
//! It re-exports and adapts error types from arkavo-protocol where needed.

pub use arkavo_protocol::error::{A2aError, Result as A2aResult};
use thiserror::Error;

/// Errors that can occur in session management
#[derive(Error, Debug)]
pub enum SessionError {
    /// Session not found
    #[error("session not found: {0}")]
    NotFound(String),

    /// Session expired
    #[error("session expired: {0}")]
    Expired(String),

    /// Connection error
    #[error("connection error: {0}")]
    Connection(String),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Database error
    #[error("database error: {0}")]
    Database(String),

    /// Transport error
    #[error("transport error: {0}")]
    Transport(String),

    /// TLS error
    #[error("TLS error: {0}")]
    Tls(String),

    /// Invalid endpoint
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),

    /// WebSocket error
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// HTTP error
    #[error("HTTP error: {0}")]
    Http(String),

    /// Not connected
    #[error("not connected")]
    NotConnected,

    /// Timeout
    #[error("timeout after {0}ms")]
    Timeout(u64),

    /// Rate limit exceeded
    #[error("rate limit exceeded")]
    RateLimitExceeded,

    /// Invalid request
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// Session already exists
    #[error("session already exists: {0}")]
    SessionAlreadyExists(String),

    /// Session closed
    #[error("session closed: {0}")]
    SessionClosed(String),

    /// Message send failed
    #[error("message send failed: {0}")]
    MessageSendFailed(String),

    /// LLM error
    #[error("LLM error: {0}")]
    LlmError(String),

    /// No LLM adapter available
    #[error("no LLM adapter available")]
    NoLlmAdapter,

    /// Generic error
    #[error("session error: {0}")]
    Other(String),
}

/// Result type for session operations
pub type Result<T> = std::result::Result<T, SessionError>;

impl From<A2aError> for SessionError {
    fn from(err: A2aError) -> Self {
        match err {
            A2aError::Transport(msg) => SessionError::Transport(msg),
            A2aError::ConnectionFailed(msg) => SessionError::Connection(msg),
            A2aError::Timeout(ms) => SessionError::Timeout(ms),
            A2aError::Tls(msg) => SessionError::Tls(msg),
            A2aError::InvalidEndpoint(msg) => SessionError::InvalidEndpoint(msg),
            A2aError::WebSocket(msg) => SessionError::WebSocket(msg),
            A2aError::Http(msg) => SessionError::Http(msg),
            A2aError::NotConnected => SessionError::NotConnected,
            A2aError::RateLimitExceeded => SessionError::RateLimitExceeded,
            A2aError::InvalidRequest(msg) => SessionError::InvalidRequest(msg),
            A2aError::SessionNotFound(msg) => SessionError::NotFound(msg),
            A2aError::SessionAlreadyExists(msg) => SessionError::SessionAlreadyExists(msg),
            A2aError::SessionClosed(msg) => SessionError::SessionClosed(msg),
            A2aError::MessageSendFailed(msg) => SessionError::MessageSendFailed(msg),
            A2aError::LlmError(msg) => SessionError::LlmError(msg),
            A2aError::NoLlmAdapter => SessionError::NoLlmAdapter,
            A2aError::Internal(msg) => SessionError::Other(msg),
            _ => SessionError::Other(err.to_string()),
        }
    }
}

impl From<SessionError> for A2aError {
    fn from(err: SessionError) -> Self {
        match err {
            SessionError::Transport(msg) => A2aError::Transport(msg),
            SessionError::Connection(msg) => A2aError::ConnectionFailed(msg),
            SessionError::Timeout(ms) => A2aError::Timeout(ms),
            SessionError::Tls(msg) => A2aError::Tls(msg),
            SessionError::InvalidEndpoint(msg) => A2aError::InvalidEndpoint(msg),
            SessionError::WebSocket(msg) => A2aError::WebSocket(msg),
            SessionError::Http(msg) => A2aError::Http(msg),
            SessionError::NotConnected => A2aError::NotConnected,
            SessionError::RateLimitExceeded => A2aError::RateLimitExceeded,
            SessionError::InvalidRequest(msg) => A2aError::InvalidRequest(msg),
            SessionError::NotFound(msg) => A2aError::SessionNotFound(msg),
            SessionError::SessionAlreadyExists(msg) => A2aError::SessionAlreadyExists(msg),
            SessionError::SessionClosed(msg) => A2aError::SessionClosed(msg),
            SessionError::MessageSendFailed(msg) => A2aError::MessageSendFailed(msg),
            SessionError::LlmError(msg) => A2aError::LlmError(msg),
            SessionError::NoLlmAdapter => A2aError::NoLlmAdapter,
            SessionError::Database(msg) => A2aError::Internal(msg),
            SessionError::Other(msg) => A2aError::Internal(msg),
            SessionError::Expired(msg) => A2aError::SessionClosed(msg),
            SessionError::Serialization(err) => A2aError::Serialization(err),
        }
    }
}

impl From<anyhow::Error> for SessionError {
    fn from(err: anyhow::Error) -> Self {
        SessionError::Other(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion() {
        let a2a_err = A2aError::NotConnected;
        let session_err: SessionError = a2a_err.into();
        assert!(matches!(session_err, SessionError::NotConnected));

        let session_err = SessionError::NotFound("test".to_string());
        let a2a_err: A2aError = session_err.into();
        assert!(matches!(a2a_err, A2aError::SessionNotFound(_)));
    }

    #[test]
    fn test_error_display() {
        let err = SessionError::NotFound("abc123".to_string());
        assert_eq!(err.to_string(), "session not found: abc123");

        let err = SessionError::Connection("timeout".to_string());
        assert_eq!(err.to_string(), "connection error: timeout");
    }
}
