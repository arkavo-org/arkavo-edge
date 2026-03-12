use anthropic_agent_sdk::ClaudeAgentOptions;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
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

    /// Use OAuth authentication (for Claude Max/Pro subscribers)
    /// If false, uses ANTHROPIC_API_KEY environment variable
    #[serde(default = "default_use_oauth")]
    pub use_oauth: bool,

    /// Maximum budget in USD for a single Claude Code session
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,

    /// Maximum tokens for extended thinking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_thinking_tokens: Option<u32>,

    /// Enable sandbox mode for command execution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_enabled: Option<bool>,

    /// Permission mode: "default", "plan", "acceptEdits"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,

    /// SDK-level tool allowlist (Claude Code tool names like "Read", "Bash", etc.)
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// SDK-level tool denylist
    #[serde(default)]
    pub disallowed_tools: Vec<String>,

    /// Use bidirectional ClaudeSDKClient instead of one-shot query()
    #[serde(default = "default_true")]
    pub use_bidirectional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
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
            use_oauth: default_use_oauth(),
            max_budget_usd: None,
            max_thinking_tokens: None,
            sandbox_enabled: None,
            permission_mode: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            use_bidirectional: true,
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
                "Workspace root does not exist: {}",
                self.workspace_root.display()
            )));
        }

        if !self.workspace_root.is_dir() {
            return Err(crate::ClaudeCodeError::Configuration(format!(
                "Workspace root is not a directory: {}",
                self.workspace_root.display()
            )));
        }

        if self.budget_tokens == 0 {
            return Err(crate::ClaudeCodeError::Configuration(
                "Budget tokens must be greater than 0".to_string(),
            ));
        }

        if let Some(budget) = self.max_budget_usd
            && budget <= 0.0
        {
            return Err(crate::ClaudeCodeError::Configuration(
                "max_budget_usd must be positive".to_string(),
            ));
        }

        if let Some(mode) = &self.permission_mode
            && !matches!(mode.as_str(), "default" | "plan" | "acceptEdits")
        {
            return Err(crate::ClaudeCodeError::Configuration(format!(
                "Invalid permission_mode: {mode}. Must be default, plan, or acceptEdits"
            )));
        }

        Ok(())
    }

    /// Build `ClaudeAgentOptions` from this config.
    ///
    /// Constructs the full SDK options including budget, sandbox, permission mode,
    /// and tool filtering. Hooks and permission callbacks are added separately
    /// by the `SdkBridge`.
    pub fn build_agent_options(&self) -> ClaudeAgentOptions {
        use anthropic_agent_sdk::ToolName;

        let allowed: Vec<ToolName> = self
            .allowed_tools
            .iter()
            .map(|s| ToolName::from(s.clone()))
            .collect();
        let disallowed: Vec<ToolName> = self
            .disallowed_tools
            .iter()
            .map(|s| ToolName::from(s.clone()))
            .collect();

        ClaudeAgentOptions {
            cwd: Some(self.workspace_root.clone()),
            max_budget_usd: self.max_budget_usd,
            max_thinking_tokens: self.max_thinking_tokens,
            sandbox: self.sandbox_enabled.map(|enabled| {
                anthropic_agent_sdk::types::SandboxSettings {
                    enabled: Some(enabled),
                    ..Default::default()
                }
            }),
            permission_mode: self.permission_mode.as_deref().map(|mode| match mode {
                "plan" => anthropic_agent_sdk::PermissionMode::Plan,
                "acceptEdits" => anthropic_agent_sdk::PermissionMode::AcceptEdits,
                _ => anthropic_agent_sdk::PermissionMode::Default,
            }),
            allowed_tools: allowed,
            disallowed_tools: disallowed,
            ..ClaudeAgentOptions::default()
        }
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

fn default_use_oauth() -> bool {
    // Default to OAuth if no API key is set
    std::env::var("ANTHROPIC_API_KEY").is_err()
}
