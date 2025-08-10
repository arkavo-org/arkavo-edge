use thiserror::Error;

#[derive(Error, Debug)]
pub enum A2aError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Request timeout after {0}ms")]
    Timeout(u64),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Auth error: {0}")]
    Auth(String),

    #[error("Invalid endpoint: {0}")]
    InvalidEndpoint(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Service discovery failed: {0}")]
    ServiceDiscovery(String),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("File transfer error: {0}")]
    FileTransfer(String),

    #[error("Transport not connected")]
    NotConnected,

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Internal error: {0}")]
    Internal(String),

    // Chat session specific errors
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Session already exists: {0}")]
    SessionAlreadyExists(String),

    #[error("Session closed: {0}")]
    SessionClosed(String),

    #[error("Message send failed: {0}")]
    MessageSendFailed(String),

    #[error("LLM error: {0}")]
    LlmError(String),

    #[error("No LLM adapter available")]
    NoLlmAdapter,
}

impl A2aError {
    pub fn to_json_rpc_code(&self) -> i32 {
        match self {
            Self::Transport(_) => crate::transport::error_codes::TRANSPORT_ERROR,
            Self::ConnectionFailed(_) => crate::transport::error_codes::CONNECTION_ERROR,
            Self::Timeout(_) => crate::transport::error_codes::TIMEOUT_ERROR,
            Self::Tls(_) => crate::transport::error_codes::TLS_ERROR,
            Self::AuthenticationFailed(_) => crate::transport::error_codes::AUTHENTICATION_ERROR,
            Self::Auth(_) => crate::transport::error_codes::AUTHENTICATION_ERROR,
            Self::InvalidEndpoint(_) => crate::transport::error_codes::INVALID_REQUEST,
            Self::Protocol(_) => crate::transport::error_codes::INVALID_REQUEST,
            Self::Configuration(_) => crate::transport::error_codes::INVALID_REQUEST,
            Self::Serialization(_) => crate::transport::error_codes::PARSE_ERROR,
            Self::WebSocket(_) => crate::transport::error_codes::TRANSPORT_ERROR,
            Self::Http(_) => crate::transport::error_codes::TRANSPORT_ERROR,
            Self::ServiceDiscovery(_) => crate::transport::error_codes::SERVER_ERROR_START - 1,
            Self::AgentNotFound(_) => crate::transport::error_codes::SERVER_ERROR_START - 2,
            Self::InvalidResponse(_) => crate::transport::error_codes::INVALID_REQUEST,
            Self::InvalidRequest(_) => crate::transport::error_codes::INVALID_REQUEST,
            Self::FileTransfer(_) => crate::transport::error_codes::SERVER_ERROR_START - 11,
            Self::NotConnected => crate::transport::error_codes::CONNECTION_ERROR,
            Self::Cancelled => crate::transport::error_codes::SERVER_ERROR_START - 3,
            Self::RateLimitExceeded => crate::transport::error_codes::SERVER_ERROR_START - 4,
            Self::Internal(_) => crate::transport::error_codes::INTERNAL_ERROR,

            // Chat session specific error codes
            Self::SessionNotFound(_) => crate::transport::error_codes::SERVER_ERROR_START - 5,
            Self::SessionAlreadyExists(_) => crate::transport::error_codes::SERVER_ERROR_START - 6,
            Self::SessionClosed(_) => crate::transport::error_codes::SERVER_ERROR_START - 7,
            Self::MessageSendFailed(_) => crate::transport::error_codes::SERVER_ERROR_START - 8,
            Self::LlmError(_) => crate::transport::error_codes::SERVER_ERROR_START - 9,
            Self::NoLlmAdapter => crate::transport::error_codes::SERVER_ERROR_START - 10,
        }
    }
}

pub type Result<T> = std::result::Result<T, A2aError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        let err = A2aError::Timeout(5000);
        assert_eq!(err.to_json_rpc_code(), -32002);

        let err = A2aError::AuthenticationFailed("Invalid token".to_string());
        assert_eq!(err.to_json_rpc_code(), -32004);

        let err = A2aError::Internal("Something went wrong".to_string());
        assert_eq!(err.to_json_rpc_code(), -32603);

        // Chat session specific errors
        let err = A2aError::SessionNotFound("session123".to_string());
        assert_eq!(err.to_json_rpc_code(), -32000 - 5); // -32005

        let err = A2aError::MessageSendFailed("Channel closed".to_string());
        assert_eq!(err.to_json_rpc_code(), -32000 - 8); // -32008

        let err = A2aError::NoLlmAdapter;
        assert_eq!(err.to_json_rpc_code(), -32000 - 10); // -32010
    }

    #[test]
    fn test_error_display() {
        let err = A2aError::ConnectionFailed("localhost:8080".to_string());
        assert_eq!(err.to_string(), "Connection failed: localhost:8080");

        let err = A2aError::Timeout(30000);
        assert_eq!(err.to_string(), "Request timeout after 30000ms");

        let err = A2aError::SessionNotFound("abc123".to_string());
        assert_eq!(err.to_string(), "Session not found: abc123");

        let err = A2aError::LlmError("Model not loaded".to_string());
        assert_eq!(err.to_string(), "LLM error: Model not loaded");
    }
}
