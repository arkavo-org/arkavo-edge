use thiserror::Error;

#[derive(Error, Debug)]
pub enum EventError {
    #[error("Serialization error: {0}")]
    Serialization(String),
    
    #[error("Deserialization error: {0}")]
    Deserialization(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Invalid event: {0}")]
    InvalidEvent(String),
    
    #[error("Buffer full")]
    BufferFull,
}

impl From<bincode::Error> for EventError {
    fn from(e: bincode::Error) -> Self {
        EventError::Serialization(e.to_string())
    }
}

impl From<serde_json::Error> for EventError {
    fn from(e: serde_json::Error) -> Self {
        EventError::Serialization(e.to_string())
    }
}