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
}

impl Default for AgentAuthConfig {
    fn default() -> Self {
        Self {
            base_url: "https://100.arkavo.net".to_string(),
            timeout: Duration::from_secs(30),
            max_retries: 3,
            retry_base_delay_ms: 100,
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

    /// Build a configuration from the environment.
    ///
    /// `ARKAVO_AUTH_URL` overrides the default base URL; `ARKAVO_AUTH_TIMEOUT_SECS`
    /// overrides the request timeout. Both fall back to defaults when unset or
    /// unparsable.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(url) = std::env::var("ARKAVO_AUTH_URL") {
            config.base_url = url;
        }

        if let Ok(secs) = std::env::var("ARKAVO_AUTH_TIMEOUT_SECS")
            && let Ok(secs) = secs.parse::<u64>()
        {
            config.timeout = Duration::from_secs(secs);
        }

        config
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::test_helpers::TEST_LOCK;

    #[tokio::test]
    async fn from_env_overrides_base_url_and_timeout() {
        let _guard = TEST_LOCK.lock().await;

        // SAFETY: serialized by TEST_LOCK, so no other test in this crate
        // reads or writes these vars concurrently.
        unsafe {
            std::env::set_var("ARKAVO_AUTH_URL", "https://agent-auth.example.test");
            std::env::set_var("ARKAVO_AUTH_TIMEOUT_SECS", "7");
        }

        let config = AgentAuthConfig::from_env();

        unsafe {
            std::env::remove_var("ARKAVO_AUTH_URL");
            std::env::remove_var("ARKAVO_AUTH_TIMEOUT_SECS");
        }

        assert_eq!(config.base_url, "https://agent-auth.example.test");
        assert_eq!(config.timeout, Duration::from_secs(7));
    }

    #[tokio::test]
    async fn from_env_falls_back_to_defaults_when_unset() {
        let _guard = TEST_LOCK.lock().await;

        // SAFETY: serialized by TEST_LOCK; ensure a clean slate before reading.
        unsafe {
            std::env::remove_var("ARKAVO_AUTH_URL");
            std::env::remove_var("ARKAVO_AUTH_TIMEOUT_SECS");
        }

        let config = AgentAuthConfig::from_env();
        let default = AgentAuthConfig::default();

        assert_eq!(config.base_url, default.base_url);
        assert_eq!(config.timeout, default.timeout);
    }
}
