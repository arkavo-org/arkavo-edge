use std::path::PathBuf;
use std::time::Duration;

/// Configuration for the agent authentication client.
#[derive(Debug, Clone)]
pub struct AgentAuthConfig {
    /// Base URL for the authentication API.
    pub base_url: String,
    /// Request timeout.
    pub timeout: Duration,
    /// Number of retry attempts.
    pub max_retries: u32,
    /// Base delay for exponential backoff (milliseconds).
    pub retry_base_delay_ms: u64,
    /// Where the token is kept. `None` is the platform's data directory.
    pub token_path: Option<PathBuf>,
}

impl Default for AgentAuthConfig {
    fn default() -> Self {
        Self {
            // The agent endpoints (`/agents/challenge`, `/agents/token`) are
            // identity's, not the KAS host's.
            base_url: "https://identity.arkavo.net".to_string(),
            timeout: Duration::from_secs(30),
            max_retries: 3,
            retry_base_delay_ms: 100,
            token_path: None,
        }
    }
}

impl AgentAuthConfig {
    /// Create a new configuration with the given base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            ..Default::default()
        }
    }

    /// Set the request timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the maximum number of retries.
    #[must_use]
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Keep the token at `path` instead of the platform's data directory.
    #[must_use]
    pub fn with_token_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.token_path = Some(path.into());
        self
    }
}
