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

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use std::sync::Mutex;

    // Env mutations must be serialized so parallel tests in this binary do not
    // observe a half-modified environment.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            unsafe { std::env::set_var(key, value) };
            Self { key, original }
        }

        fn remove(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            unsafe { std::env::remove_var(key) };
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.original {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[spec("MCPC-001")]
    #[test]
    fn test_auth_available_with_api_key() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _key = EnvGuard::set("ANTHROPIC_API_KEY", "test-key-for-auth-check");
        let _session = EnvGuard::remove("CLAUDE_CODE_SESSION_ACCESS_TOKEN");

        assert!(is_auth_available());
    }

    #[spec("MCPC-001")]
    #[test]
    fn test_auth_available_with_session_token() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _key = EnvGuard::remove("ANTHROPIC_API_KEY");
        let _session = EnvGuard::set("CLAUDE_CODE_SESSION_ACCESS_TOKEN", "test-session-token");

        assert!(is_auth_available());
    }

    #[spec("MCPC-008")]
    #[test]
    fn test_error_display_includes_user_message() {
        let cases: Vec<(ClaudeCodeError, &str)> = vec![
            (
                ClaudeCodeError::Sdk("connection reset".to_string()),
                "SDK error: connection reset",
            ),
            (
                ClaudeCodeError::Auth("token expired".to_string()),
                "Authentication error: token expired",
            ),
            (
                ClaudeCodeError::PolicyViolation("blocked content".to_string()),
                "Policy violation: blocked content",
            ),
            (
                ClaudeCodeError::Configuration("bad field".to_string()),
                "Configuration error: bad field",
            ),
            (
                ClaudeCodeError::BudgetExceeded("spent 2.00 of 1.00 USD".to_string()),
                "Budget exceeded: spent 2.00 of 1.00 USD",
            ),
            (
                ClaudeCodeError::Other("misc".to_string()),
                "Other error: misc",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[spec("MCPC-008")]
    #[test]
    fn test_error_classification_from_conversions() {
        let json_error = serde_json::from_str::<serde_json::Value>("{not valid json").unwrap_err();
        let classified: ClaudeCodeError = json_error.into();
        assert!(
            matches!(classified, ClaudeCodeError::Json(_)),
            "serde_json::Error must classify as Json variant, got {classified:?}"
        );

        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing file");
        let classified: ClaudeCodeError = io_error.into();
        assert!(
            matches!(classified, ClaudeCodeError::Io(_)),
            "io::Error must classify as Io variant, got {classified:?}"
        );
    }
}
