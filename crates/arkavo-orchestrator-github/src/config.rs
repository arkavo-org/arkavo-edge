use crate::error::{Error, Result};
use arkavo_observability::config::{ConfigValidator, SecureConfigProvider};
use std::env;
use std::sync::Arc;

#[derive(Clone)]
pub struct OrchestratorConfig {
    pub webhook_secret: String,
    pub github_app_id: String,
    pub github_app_private_key: String,
    pub rate_limit_requests_per_second: u32,
    pub rate_limit_burst_size: u32,
    pub metrics_port: u16,
    pub webhook_port: u16,
    pub max_request_body_size: usize,
}

impl OrchestratorConfig {
    pub fn from_env() -> Result<Self> {
        let provider = Self::build_config_provider()?;

        let webhook_secret = provider
            .get_config("ARKAVO_GITHUB_WEBHOOK_SECRET")
            .map_err(|e| Error::Other(anyhow::anyhow!("Config error: {e}")))?
            .ok_or_else(|| Error::Other(anyhow::anyhow!("Missing ARKAVO_GITHUB_WEBHOOK_SECRET")))?;

        let github_app_id = provider
            .get_config("ARKAVO_GITHUB_APP_ID")
            .map_err(|e| Error::Other(anyhow::anyhow!("Config error: {e}")))?
            .ok_or_else(|| Error::Other(anyhow::anyhow!("Missing ARKAVO_GITHUB_APP_ID")))?;

        let github_app_private_key = provider
            .get_config("ARKAVO_GITHUB_APP_PRIVATE_KEY")
            .map_err(|e| Error::Other(anyhow::anyhow!("Config error: {e}")))?
            .ok_or_else(|| {
                Error::Other(anyhow::anyhow!("Missing ARKAVO_GITHUB_APP_PRIVATE_KEY"))
            })?;

        let rate_limit_requests_per_second = env::var("ARKAVO_RATE_LIMIT_RPS")
            .unwrap_or_else(|_| "100".to_string())
            .parse()
            .map_err(|e| Error::Other(anyhow::anyhow!("Invalid ARKAVO_RATE_LIMIT_RPS: {e}")))?;

        let rate_limit_burst_size = env::var("ARKAVO_RATE_LIMIT_BURST")
            .unwrap_or_else(|_| "200".to_string())
            .parse()
            .map_err(|e| Error::Other(anyhow::anyhow!("Invalid ARKAVO_RATE_LIMIT_BURST: {e}")))?;

        let metrics_port = env::var("ARKAVO_METRICS_PORT")
            .unwrap_or_else(|_| "9090".to_string())
            .parse()
            .map_err(|e| Error::Other(anyhow::anyhow!("Invalid ARKAVO_METRICS_PORT: {e}")))?;

        let webhook_port = env::var("ARKAVO_WEBHOOK_PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .map_err(|e| Error::Other(anyhow::anyhow!("Invalid ARKAVO_WEBHOOK_PORT: {e}")))?;

        let max_request_body_size = env::var("ARKAVO_MAX_REQUEST_BODY_SIZE")
            .unwrap_or_else(|_| "10485760".to_string())
            .parse()
            .map_err(|e| {
                Error::Other(anyhow::anyhow!("Invalid ARKAVO_MAX_REQUEST_BODY_SIZE: {e}"))
            })?;

        Ok(Self {
            webhook_secret,
            github_app_id,
            github_app_private_key,
            rate_limit_requests_per_second,
            rate_limit_burst_size,
            metrics_port,
            webhook_port,
            max_request_body_size,
        })
    }

    fn build_config_provider() -> Result<Arc<SecureConfigProvider>> {
        let mut provider = SecureConfigProvider::new();

        let webhook_secret = env::var("ARKAVO_GITHUB_WEBHOOK_SECRET")
            .map_err(|_| Error::Other(anyhow::anyhow!("Missing ARKAVO_GITHUB_WEBHOOK_SECRET")))?;

        let github_app_id = env::var("ARKAVO_GITHUB_APP_ID")
            .map_err(|_| Error::Other(anyhow::anyhow!("Missing ARKAVO_GITHUB_APP_ID")))?;

        let github_app_private_key = env::var("ARKAVO_GITHUB_APP_PRIVATE_KEY")
            .map_err(|_| Error::Other(anyhow::anyhow!("Missing ARKAVO_GITHUB_APP_PRIVATE_KEY")))?;

        let mut validator = ConfigValidator::sensitive();
        validator.min_length = Some(20);
        validator.max_length = Some(256);
        validator.required = true;

        provider
            .set_config(
                "ARKAVO_GITHUB_WEBHOOK_SECRET",
                webhook_secret.as_str(),
                validator,
            )
            .map_err(|e| Error::Other(anyhow::anyhow!("Config validation failed: {e}")))?;

        let mut validator = ConfigValidator::required();
        validator.min_length = Some(1);
        validator.max_length = Some(20);

        provider
            .set_config("ARKAVO_GITHUB_APP_ID", github_app_id.as_str(), validator)
            .map_err(|e| Error::Other(anyhow::anyhow!("Config validation failed: {e}")))?;

        let mut validator = ConfigValidator::sensitive();
        validator.min_length = Some(100);
        validator.max_length = Some(10000);
        validator.required = true;

        provider
            .set_config(
                "ARKAVO_GITHUB_APP_PRIVATE_KEY",
                github_app_private_key.as_str(),
                validator,
            )
            .map_err(|e| Error::Other(anyhow::anyhow!("Config validation failed: {e}")))?;

        Ok(Arc::new(provider))
    }

    pub fn get_masked_secret(&self) -> String {
        if self.webhook_secret.len() > 8 {
            format!("{}***", &self.webhook_secret[..4])
        } else {
            "***".to_string()
        }
    }

    pub fn get_masked_private_key(&self) -> String {
        if self.github_app_private_key.len() > 20 {
            format!(
                "{}...{}",
                &self.github_app_private_key[..10],
                &self.github_app_private_key[self.github_app_private_key.len() - 10..]
            )
        } else {
            "***".to_string()
        }
    }
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            webhook_secret: String::new(),
            github_app_id: String::new(),
            github_app_private_key: String::new(),
            rate_limit_requests_per_second: 100,
            rate_limit_burst_size: 200,
            metrics_port: 9090,
            webhook_port: 3000,
            max_request_body_size: 10 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.rate_limit_requests_per_second, 100);
        assert_eq!(config.rate_limit_burst_size, 200);
        assert_eq!(config.metrics_port, 9090);
        assert_eq!(config.webhook_port, 3000);
        assert_eq!(config.max_request_body_size, 10 * 1024 * 1024);
    }

    #[test]
    fn test_masked_secret() {
        let mut config = OrchestratorConfig::default();
        config.webhook_secret = "test-secret-12345".to_string();
        assert_eq!(config.get_masked_secret(), "test***");
    }

    #[test]
    fn test_masked_private_key() {
        let mut config = OrchestratorConfig::default();
        config.github_app_private_key =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...END-----".to_string();
        let masked = config.get_masked_private_key();
        assert!(masked.contains("-----BEGIN"));
        assert!(masked.contains("..."));
        assert!(masked.contains("END-----"));
    }
}
