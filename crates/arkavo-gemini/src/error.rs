use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeminiError {
    /// Boxed because `tokio_tungstenite::tungstenite::Error` is ~136 bytes
    /// on its own; carrying it inline blows `GeminiError` past clippy's
    /// `result_large_err` threshold and forces `#[allow]` on every public
    /// `Result<_, GeminiError>` function in the crate. Manual `From` impl
    /// below preserves `?` ergonomics at callsites.
    #[error("WebSocket connection error: {0}")]
    WebSocketError(Box<tokio_tungstenite::tungstenite::Error>),

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

impl From<tokio_tungstenite::tungstenite::Error> for GeminiError {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        GeminiError::WebSocketError(Box::new(e))
    }
}
