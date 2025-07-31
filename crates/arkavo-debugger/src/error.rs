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
}

pub type Result<T> = std::result::Result<T, DebuggerError>;