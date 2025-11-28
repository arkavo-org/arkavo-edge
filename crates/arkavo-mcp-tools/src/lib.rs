#![allow(clippy::uninlined_format_args)]

pub mod browser;
pub mod code_analysis;
pub mod code_review;
pub mod filesystem;
pub mod git;
pub mod github;
pub mod github_checks;
pub mod github_org_knowledge;
pub mod github_review;
pub mod health_check;
pub mod mcp_connection;
pub mod osv;
pub mod registry;
pub mod semgrep;
pub mod server;
pub mod shell_exec;
pub mod state;
pub mod state_store;
pub mod syft;
pub mod tdf;
pub mod test_runner;
pub mod time_sync;
pub mod tui;
pub mod web_search;

#[cfg(feature = "ntp-server")]
pub mod ntp_server;

// Re-export commonly used types
pub use registry::{DetailLevel, McpClient, McpTool, MinimalToolInfo, ToolInfo, ToolRegistry};
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
