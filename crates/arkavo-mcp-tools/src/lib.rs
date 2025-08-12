#![allow(clippy::uninlined_format_args)]

pub mod code_analysis;
pub mod filesystem;
pub mod git;
pub mod github;
pub mod server;
pub mod state;
pub mod state_store;
pub mod tui;

// Re-export commonly used types
pub use server::{Tool, ToolSchema};

// Re-export error types
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Tool execution failed: {0}")]
    Execution(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ToolError>;
