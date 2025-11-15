//! KAS (Key Access Service) configuration and utilities
//!
//! This module provides configuration for connecting to the Arkavo KAS
//! for policy-based key unwrapping and access control.

use std::env;

/// Default Arkavo KAS URL for production
pub const DEFAULT_KAS_URL: &str = "https://100.arkavo.net/kas/v2/rewrap";

/// Default Arkavo Identity URL for authentication
pub const DEFAULT_IDENTITY_URL: &str = "https://identity.arkavo.net";

/// Environment variable name for KAS URL override
pub const KAS_URL_ENV: &str = "ARKAVO_KAS_URL";

/// Environment variable name for Identity URL override
pub const IDENTITY_URL_ENV: &str = "ARKAVO_IDENTITY_URL";

/// Environment variable name for KAS OAuth token
pub const KAS_TOKEN_ENV: &str = "ARKAVO_KAS_TOKEN";

/// KAS configuration
#[derive(Debug, Clone)]
pub struct KasConfig {
    /// KAS endpoint URL
    pub url: String,

    /// Identity service URL for authentication
    pub identity_url: String,

    /// OAuth token for KAS authentication (optional)
    pub token: Option<String>,
}

impl KasConfig {
    /// Create a new KAS configuration
    pub fn new(url: String, identity_url: String, token: Option<String>) -> Self {
        Self {
            url,
            identity_url,
            token,
        }
    }

    /// Create KAS configuration from environment variables
    ///
    /// Reads:
    /// - `ARKAVO_KAS_URL` (optional, defaults to production KAS)
    /// - `ARKAVO_IDENTITY_URL` (optional, defaults to production Identity)
    /// - `ARKAVO_KAS_TOKEN` (optional)
    pub fn from_env() -> Self {
        let url = env::var(KAS_URL_ENV).unwrap_or_else(|_| DEFAULT_KAS_URL.to_string());
        let identity_url =
            env::var(IDENTITY_URL_ENV).unwrap_or_else(|_| DEFAULT_IDENTITY_URL.to_string());
        let token = env::var(KAS_TOKEN_ENV).ok();

        Self {
            url,
            identity_url,
            token,
        }
    }

    /// Get the default production KAS configuration
    pub fn production() -> Self {
        Self {
            url: DEFAULT_KAS_URL.to_string(),
            identity_url: DEFAULT_IDENTITY_URL.to_string(),
            token: None,
        }
    }

    /// Create a custom KAS configuration
    pub fn custom(url: impl Into<String>, identity_url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            identity_url: identity_url.into(),
            token: None,
        }
    }

    /// Set the OAuth token
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Get the rewrap endpoint URL
    pub fn rewrap_url(&self) -> &str {
        &self.url
    }

    /// Get the identity service URL
    pub fn identity_url(&self) -> &str {
        &self.identity_url
    }
}

impl Default for KasConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_kas_config() {
        let config = KasConfig::production();
        assert_eq!(config.url, DEFAULT_KAS_URL);
        assert_eq!(config.identity_url, DEFAULT_IDENTITY_URL);
        assert!(config.token.is_none());
    }

    #[test]
    fn test_custom_kas_config() {
        let config = KasConfig::custom(
            "https://custom.kas.example.com/rewrap",
            "https://custom.identity.example.com",
        );
        assert_eq!(config.url, "https://custom.kas.example.com/rewrap");
        assert_eq!(config.identity_url, "https://custom.identity.example.com");
    }

    #[test]
    fn test_kas_config_with_token() {
        let config = KasConfig::production().with_token("test-token-123");
        assert_eq!(config.url, DEFAULT_KAS_URL);
        assert_eq!(config.identity_url, DEFAULT_IDENTITY_URL);
        assert_eq!(config.token, Some("test-token-123".to_string()));
    }

    #[test]
    fn test_rewrap_url() {
        let config = KasConfig::production();
        assert_eq!(config.rewrap_url(), DEFAULT_KAS_URL);
    }

    #[test]
    fn test_identity_url() {
        let config = KasConfig::production();
        assert_eq!(config.identity_url(), DEFAULT_IDENTITY_URL);
    }
}
