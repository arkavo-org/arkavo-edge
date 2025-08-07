use thiserror::Error;

#[derive(Error, Debug)]
pub enum DebuggerError {
    #[error("Event not found: {0}")]
    EventNotFound(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Storage error: {0}")]
    Storage(#[from] arkavo_memory::error::MemoryError),

    #[error("Event error: {0}")]
    Event(#[from] arkavo_events::EventError),

    #[error("Replay error: {0}")]
    Replay(String),

    #[error("Analysis error: {0}")]
    Analysis(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Parse error: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, DebuggerError>;

/// Alias for compatibility with SessionManager
pub type DebugError = DebuggerError;

impl From<bincode::Error> for DebuggerError {
    fn from(err: bincode::Error) -> Self {
        DebuggerError::Serialization(err.to_string())
    }
}

impl From<uuid::Error> for DebuggerError {
    fn from(err: uuid::Error) -> Self {
        DebuggerError::Parse(err.to_string())
    }
}

impl From<chrono::ParseError> for DebuggerError {
    fn from(err: chrono::ParseError) -> Self {
        DebuggerError::Parse(err.to_string())
    }
}
