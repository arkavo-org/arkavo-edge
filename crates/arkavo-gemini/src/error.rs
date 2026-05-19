use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeminiError {
    #[error("WebSocket connection error: {0}")]
    WebSocketError(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Session not connected")]
    NotConnected,

    #[error("Invalid message format: {0}")]
    InvalidMessage(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Tool execution error: {0}")]
    ToolExecutionError(String),

    #[error("Schema validation error: {0}")]
    SchemaValidationError(String),

    #[error("Connection timeout after {0}ms")]
    ConnectionTimeout(u64),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Session closed by server: {0}")]
    SessionClosed(String),

    #[error("Authentication failed: {0}")]
    AuthenticationError(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
}

pub type Result<T> = std::result::Result<T, GeminiError>;
