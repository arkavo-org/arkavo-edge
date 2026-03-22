use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodeSearchError {
    #[error("Ripgrep command failed: {0}")]
    RipgrepError(String),

    #[error("Invalid pattern: {0}")]
    InvalidPattern(String),

    #[error("Path not found: {0}")]
    PathNotFound(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Tool error: {0}")]
    ToolError(String),
}

pub type Result<T> = std::result::Result<T, CodeSearchError>;
