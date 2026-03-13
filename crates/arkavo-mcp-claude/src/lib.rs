pub mod capability;
pub mod config;
pub mod event_mapper;
pub mod hook_handler;
pub mod policy_bridge;
pub mod sdk_bridge;
pub mod tools;

pub use capability::ClaudeCodeCapability;
pub use config::{ClaudeCodeConfig, ToolPermissions};
pub use tools::register_tools;

/// Check if Claude authentication is available (API key or cached OAuth token)
///
/// Use this to avoid blocking on interactive OAuth flow in non-interactive contexts.
pub fn is_auth_available() -> bool {
    // Check for API key first
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return true;
    }

    // Inside a Claude Code session the CLI inherits the parent's session
    // token, so no separate API key or OAuth flow is needed
    if std::env::var("CLAUDE_CODE_SESSION_ACCESS_TOKEN").is_ok() {
        return true;
    }

    // Check for cached OAuth token
    anthropic_agent_sdk::auth::OAuthClient::new()
        .map(|c| c.is_authenticated())
        .unwrap_or(false)
}

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
