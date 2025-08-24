use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeConfig {
    /// Enable or disable the capability
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Anthropic model to use (e.g., "claude-3-sonnet-20240229" or "deepseek-chat")
    #[serde(default = "default_model")]
    pub anthropic_model: String,

    /// Anthropic base URL (for custom endpoints like DeepSeek)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_base_url: Option<String>,

    /// Anthropic auth token (alternative to API key)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_auth_token: Option<String>,

    /// Small/fast model for simpler tasks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_small_fast_model: Option<String>,

    /// Workspace root directory for file operations
    pub workspace_root: PathBuf,

    /// Maximum tokens allowed per session
    #[serde(default = "default_budget_tokens")]
    pub budget_tokens: u32,

    /// Enable sensitive content redaction in logs
    #[serde(default = "default_log_redaction")]
    pub log_redaction: bool,

    /// Tool permissions
    #[serde(default)]
    pub tools: ToolPermissions,

    /// File access patterns (globs)
    #[serde(default)]
    pub allow_globs: Vec<String>,

    /// Deny patterns take precedence over allow patterns
    #[serde(default)]
    pub deny_globs: Vec<String>,

    /// Retry configuration
    #[serde(default)]
    pub retry: RetryConfig,

    /// Session TTL in seconds
    #[serde(default = "default_session_ttl")]
    pub session_ttl_secs: u64,

    /// Node.js runtime path (if custom)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissions {
    /// Allow file read operations
    #[serde(default = "default_true")]
    pub read: bool,

    /// Allow file write operations (default: false)
    #[serde(default)]
    pub write: bool,

    /// Allow shell command execution (default: false)
    #[serde(default)]
    pub exec: bool,

    /// Allow web search operations (default: false)
    #[serde(default)]
    pub web_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    /// Backoff duration in milliseconds
    #[serde(default = "default_backoff_ms")]
    pub backoff_ms: u64,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            anthropic_model: default_model(),
            anthropic_base_url: None,
            anthropic_auth_token: None,
            anthropic_small_fast_model: None,
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            budget_tokens: default_budget_tokens(),
            log_redaction: true,
            tools: ToolPermissions::default(),
            allow_globs: vec!["**/*.rs".to_string(), "**/*.toml".to_string()],
            deny_globs: vec![
                "**/.secrets/**".to_string(),
                "**/target/**".to_string(),
                "**/node_modules/**".to_string(),
            ],
            retry: RetryConfig::default(),
            session_ttl_secs: default_session_ttl(),
            node_path: None,
        }
    }
}

impl Default for ToolPermissions {
    fn default() -> Self {
        Self {
            read: true,
            write: false,
            exec: false,
            web_search: false,
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_ms: 800,
        }
    }
}

impl ClaudeCodeConfig {
    /// Validate the configuration
    pub fn validate(&self) -> crate::Result<()> {
        if !self.workspace_root.exists() {
            return Err(crate::ClaudeCodeError::Configuration(format!(
                "Workspace root does not exist: {:?}",
                self.workspace_root
            )));
        }

        if !self.workspace_root.is_dir() {
            return Err(crate::ClaudeCodeError::Configuration(format!(
                "Workspace root is not a directory: {:?}",
                self.workspace_root
            )));
        }

        if self.budget_tokens == 0 {
            return Err(crate::ClaudeCodeError::Configuration(
                "Budget tokens must be greater than 0".to_string(),
            ));
        }

        Ok(())
    }

    /// Check if a path is allowed based on glob patterns
    pub fn is_path_allowed(&self, path: &str) -> bool {
        // First check deny patterns (they take precedence)
        for pattern in &self.deny_globs {
            if glob::Pattern::new(pattern)
                .ok()
                .is_some_and(|p| p.matches(path))
            {
                return false;
            }
        }

        // If no allow patterns specified, allow all (except denied)
        if self.allow_globs.is_empty() {
            return true;
        }

        // Check allow patterns
        for pattern in &self.allow_globs {
            if glob::Pattern::new(pattern)
                .ok()
                .is_some_and(|p| p.matches(path))
            {
                return true;
            }
        }

        false
    }
}

fn default_enabled() -> bool {
    true
}

fn default_model() -> String {
    "claude-3-sonnet-20240229".to_string()
}

fn default_budget_tokens() -> u32 {
    200_000
}

fn default_log_redaction() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_max_attempts() -> u32 {
    3
}

fn default_backoff_ms() -> u64 {
    800
}

fn default_session_ttl() -> u64 {
    3600
}
