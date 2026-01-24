pub mod capability;
pub mod config;
pub mod event_mapper;
pub mod policy_bridge;
pub mod sdk_bridge;
pub mod tools;

pub use capability::ClaudeCodeCapability;
pub use config::{ClaudeCodeConfig, ToolPermissions};
pub use tools::register_tools;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClaudeCodeError {
    #[error("SDK error: {0}")]
    Sdk(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Policy violation: {0}")]
    PolicyViolation(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

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
