pub mod capability;
pub mod config;
pub mod event_mapper;
pub mod jsonrpc;
pub mod node_bridge;
pub mod policy_bridge;
pub mod tools;

pub use capability::ClaudeCodeCapability;
pub use config::{ClaudeCodeConfig, ToolPermissions};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClaudeCodeError {
    #[error("Node.js subprocess error: {0}")]
    NodeProcess(String),

    #[error("JSON-RPC error: {0}")]
    JsonRpc(String),

    #[error("Policy violation: {0}")]
    PolicyViolation(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("SDK error: {0}")]
    Sdk(String),

    #[error("Budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ClaudeCodeError>;
