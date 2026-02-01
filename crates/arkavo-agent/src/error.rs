//! Error types for the arkavo-agent crate

use thiserror::Error;

/// Errors that can occur in agent lifecycle management
#[derive(Error, Debug)]
pub enum AgentError {
    /// Configuration parsing or validation error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Agent registration error
    #[error("Registration error: {0}")]
    Registration(String),

    /// Agent discovery error
    #[error("Discovery error: {0}")]
    Discovery(String),

    /// Agent not found in registry
    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    /// Authentication or verification failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Invalid challenge or expired
    #[error("Invalid challenge: {0}")]
    InvalidChallenge(String),

    /// I/O error during operations
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Crypto error (wrapper around arkavo-crypto errors)
    #[error("Crypto error: {0}")]
    Crypto(String),

    /// General internal error
    #[error("Internal error: {0}")]
    Internal(String),

    /// Protocol error (wrapper around arkavo-protocol errors)
    #[error("Protocol error: {0}")]
    Protocol(String),
}

impl From<arkavo_protocol::A2aError> for AgentError {
    fn from(err: arkavo_protocol::A2aError) -> Self {
        use arkavo_protocol::A2aError;
        match err {
            A2aError::Configuration(msg) => AgentError::Config(msg),
            A2aError::AuthenticationFailed(msg) => AgentError::AuthenticationFailed(msg),
            A2aError::InvalidRequest(msg) => AgentError::Registration(msg),
            A2aError::AgentNotFound(msg) => AgentError::AgentNotFound(msg),
            A2aError::Transport(msg) => AgentError::Discovery(msg),
            A2aError::ServiceDiscovery(msg) => AgentError::Discovery(msg),
            A2aError::Internal(msg) => AgentError::Internal(msg),
            A2aError::InvalidEndpoint(msg) => AgentError::Internal(msg),
            A2aError::ConnectionFailed(_) => AgentError::Internal("Connection failed".to_string()),
            A2aError::Timeout(_) => AgentError::Internal("Timeout".to_string()),
            A2aError::Protocol(msg) => AgentError::Protocol(msg),
            A2aError::Tls(msg) => AgentError::Internal(format!("TLS error: {msg}")),
            A2aError::Http(msg) => AgentError::Internal(format!("HTTP error: {msg}")),
            A2aError::Serialization(_) => AgentError::Internal("Serialization error".to_string()),
            A2aError::Auth(msg) => AgentError::AuthenticationFailed(msg),
            A2aError::WebSocket(msg) => AgentError::Internal(format!("WebSocket: {msg}")),
            A2aError::InvalidResponse(msg) => {
                AgentError::Internal(format!("Invalid response: {msg}"))
            }
            A2aError::FileTransfer(msg) => AgentError::Internal(format!("File transfer: {msg}")),
            A2aError::NotConnected => AgentError::Internal("Not connected".to_string()),
            A2aError::Cancelled => AgentError::Internal("Cancelled".to_string()),
            A2aError::RateLimitExceeded => AgentError::Internal("Rate limit exceeded".to_string()),
            A2aError::SessionNotFound(msg) => AgentError::AgentNotFound(msg),
            A2aError::SessionAlreadyExists(msg) => {
                AgentError::Internal(format!("Session exists: {msg}"))
            }
            A2aError::SessionClosed(msg) => AgentError::Internal(format!("Session closed: {msg}")),
            A2aError::MessageSendFailed(msg) => {
                AgentError::Internal(format!("Message send failed: {msg}"))
            }
            A2aError::LlmError(msg) => AgentError::Internal(format!("LLM error: {msg}")),
            A2aError::NoLlmAdapter => AgentError::Internal("No LLM adapter".to_string()),
        }
    }
}

/// Result type alias for agent operations
pub type Result<T> = std::result::Result<T, AgentError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = AgentError::Config("missing field".to_string());
        assert_eq!(err.to_string(), "Configuration error: missing field");

        let err = AgentError::AgentNotFound("agent-123".to_string());
        assert_eq!(err.to_string(), "Agent not found: agent-123");

        let err = AgentError::AuthenticationFailed("invalid signature".to_string());
        assert_eq!(err.to_string(), "Authentication failed: invalid signature");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let agent_err: AgentError = io_err.into();
        assert!(matches!(agent_err, AgentError::Io(_)));
        assert!(agent_err.to_string().contains("file missing"));
    }
}
