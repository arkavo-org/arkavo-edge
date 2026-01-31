//! Security configuration and TLS settings

use crate::error::{Result, SecurityError};
use std::fs;
use std::path::Path;
use tracing::debug;

/// Security configuration
#[derive(Debug, Clone, Default)]
pub struct SecurityConfig {
    /// TLS settings
    pub tls: TlsSettings,
    /// Authentication settings
    pub authentication: AuthSettings,
    /// Rate limiting settings
    pub rate_limiting: RateLimitSettings,
}

/// TLS settings
#[derive(Debug, Clone)]
pub struct TlsSettings {
    /// Whether TLS is enabled
    pub enabled: bool,
    /// Whether to verify certificates
    pub verify_certificates: bool,
    /// Minimum TLS version
    pub minimum_tls_version: TlsVersion,
    /// Client certificate path
    pub client_cert_path: Option<String>,
    /// Client key path
    pub client_key_path: Option<String>,
    /// CA certificate path
    pub ca_cert_path: Option<String>,
}

impl Default for TlsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            verify_certificates: true,
            minimum_tls_version: TlsVersion::Tls12,
            client_cert_path: None,
            client_key_path: None,
            ca_cert_path: None,
        }
    }
}

/// TLS version
#[derive(Debug, Clone, Copy)]
pub enum TlsVersion {
    /// TLS 1.0
    Tls10,
    /// TLS 1.1
    Tls11,
    /// TLS 1.2
    Tls12,
    /// TLS 1.3
    Tls13,
}

impl TlsVersion {
    /// Get the TLS version as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tls10 => "TLS 1.0",
            Self::Tls11 => "TLS 1.1",
            Self::Tls12 => "TLS 1.2",
            Self::Tls13 => "TLS 1.3",
        }
    }
}

/// Authentication settings
#[derive(Debug, Clone)]
pub struct AuthSettings {
    /// Authentication method
    pub method: AuthMethod,
    /// Bearer token
    pub token: Option<String>,
    /// API key
    pub api_key: Option<String>,
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            method: AuthMethod::None,
            token: None,
            api_key: None,
        }
    }
}

/// Authentication method
#[derive(Debug, Clone, Copy)]
pub enum AuthMethod {
    /// No authentication
    None,
    /// Bearer token
    Bearer,
    /// API key
    ApiKey,
    /// Mutual TLS
    MutualTls,
}

/// Rate limit settings
#[derive(Debug, Clone)]
pub struct RateLimitSettings {
    /// Whether rate limiting is enabled
    pub enabled: bool,
    /// Requests per second
    pub requests_per_second: u32,
    /// Burst size
    pub burst_size: u32,
}

impl Default for RateLimitSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            requests_per_second: 100,
            burst_size: 200,
        }
    }
}

impl SecurityConfig {
    /// Create a new security config builder
    pub fn builder() -> SecurityConfigBuilder {
        SecurityConfigBuilder::default()
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if self.tls.enabled
            && self.tls.client_cert_path.is_some()
            && self.tls.client_key_path.is_none()
        {
            return Err(SecurityError::Tls(
                "Client certificate provided but client key is missing".to_string(),
            ));
        }

        if let Some(cert_path) = &self.tls.client_cert_path
            && !Path::new(cert_path).exists()
        {
            return Err(SecurityError::Tls(format!(
                "Client certificate file not found: {cert_path}"
            )));
        }

        if let Some(key_path) = &self.tls.client_key_path
            && !Path::new(key_path).exists()
        {
            return Err(SecurityError::Tls(format!(
                "Client key file not found: {key_path}"
            )));
        }

        Ok(())
    }

    /// Load certificates from files
    pub fn load_certificates(&self) -> Result<CertificateData> {
        let mut data = CertificateData::default();

        if let Some(cert_path) = &self.tls.client_cert_path {
            debug!("Loading client certificate from: {}", cert_path);
            data.client_cert = Some(
                fs::read_to_string(cert_path)
                    .map_err(|e| SecurityError::Tls(format!("Failed to read client cert: {e}")))?,
            );
        }

        if let Some(key_path) = &self.tls.client_key_path {
            debug!("Loading client key from: {}", key_path);
            data.client_key = Some(
                fs::read_to_string(key_path)
                    .map_err(|e| SecurityError::Tls(format!("Failed to read client key: {e}")))?,
            );
        }

        if let Some(ca_path) = &self.tls.ca_cert_path {
            debug!("Loading CA certificate from: {}", ca_path);
            data.ca_cert = Some(
                fs::read_to_string(ca_path)
                    .map_err(|e| SecurityError::Tls(format!("Failed to read CA cert: {e}")))?,
            );
        }

        Ok(data)
    }
}

/// Certificate data
#[derive(Default)]
pub struct CertificateData {
    /// Client certificate
    pub client_cert: Option<String>,
    /// Client key
    pub client_key: Option<String>,
    /// CA certificate
    pub ca_cert: Option<String>,
}

/// Security config builder
#[derive(Default)]
pub struct SecurityConfigBuilder {
    config: SecurityConfig,
}

impl SecurityConfigBuilder {
    /// Set TLS enabled
    pub fn tls_enabled(mut self, enabled: bool) -> Self {
        self.config.tls.enabled = enabled;
        self
    }

    /// Set certificate verification
    pub fn verify_certificates(mut self, verify: bool) -> Self {
        self.config.tls.verify_certificates = verify;
        self
    }

    /// Set minimum TLS version
    pub fn minimum_tls_version(mut self, version: TlsVersion) -> Self {
        self.config.tls.minimum_tls_version = version;
        self
    }

    /// Set client certificates
    pub fn client_certificates(mut self, cert_path: String, key_path: String) -> Self {
        self.config.tls.client_cert_path = Some(cert_path);
        self.config.tls.client_key_path = Some(key_path);
        self
    }

    /// Set CA certificate
    pub fn ca_certificate(mut self, ca_path: String) -> Self {
        self.config.tls.ca_cert_path = Some(ca_path);
        self
    }

    /// Set authentication method
    pub fn auth_method(mut self, method: AuthMethod) -> Self {
        self.config.authentication.method = method;
        self
    }

    /// Set bearer token
    pub fn bearer_token(mut self, token: String) -> Self {
        self.config.authentication.method = AuthMethod::Bearer;
        self.config.authentication.token = Some(token);
        self
    }

    /// Set API key
    pub fn api_key(mut self, key: String) -> Self {
        self.config.authentication.method = AuthMethod::ApiKey;
        self.config.authentication.api_key = Some(key);
        self
    }

    /// Set rate limiting
    pub fn rate_limiting(mut self, enabled: bool, rps: u32, burst: u32) -> Self {
        self.config.rate_limiting = RateLimitSettings {
            enabled,
            requests_per_second: rps,
            burst_size: burst,
        };
        self
    }

    /// Build the configuration
    pub fn build(self) -> Result<SecurityConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn test_default_security_config() {
        let config = SecurityConfig::default();
        assert!(config.tls.enabled);
        assert!(config.tls.verify_certificates);
        assert!(matches!(config.authentication.method, AuthMethod::None));
    }

    #[test]
    fn test_security_config_builder() {
        let config = SecurityConfig::builder()
            .tls_enabled(true)
            .verify_certificates(false)
            .minimum_tls_version(TlsVersion::Tls13)
            .bearer_token("test-token".to_string())
            .rate_limiting(true, 50, 100)
            .build()
            .unwrap();

        assert!(config.tls.enabled);
        assert!(!config.tls.verify_certificates);
        assert!(matches!(config.tls.minimum_tls_version, TlsVersion::Tls13));
        assert!(matches!(config.authentication.method, AuthMethod::Bearer));
        assert_eq!(config.authentication.token, Some("test-token".to_string()));
        assert_eq!(config.rate_limiting.requests_per_second, 50);
    }

    #[test]
    fn test_validation_missing_key() {
        let mut config = SecurityConfig::default();
        config.tls.client_cert_path = Some("/path/to/cert".to_string());

        let result = config.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("client key is missing")
        );
    }
}
