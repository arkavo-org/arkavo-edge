use crate::error::Result;
use std::fs;
use std::path::Path;
use tracing::debug;

#[derive(Debug, Clone, Default)]
pub struct SecurityConfig {
    pub tls: TlsSettings,
    pub authentication: AuthSettings,
    pub rate_limiting: RateLimitSettings,
}

#[derive(Debug, Clone)]
pub struct TlsSettings {
    pub enabled: bool,
    pub verify_certificates: bool,
    pub minimum_tls_version: TlsVersion,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
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

#[derive(Debug, Clone, Copy)]
pub enum TlsVersion {
    Tls10,
    Tls11,
    Tls12,
    Tls13,
}

impl TlsVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tls10 => "TLS 1.0",
            Self::Tls11 => "TLS 1.1",
            Self::Tls12 => "TLS 1.2",
            Self::Tls13 => "TLS 1.3",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthSettings {
    pub method: AuthMethod,
    pub token: Option<String>,
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

#[derive(Debug, Clone, Copy)]
pub enum AuthMethod {
    None,
    Bearer,
    ApiKey,
    MutualTls,
}

#[derive(Debug, Clone)]
pub struct RateLimitSettings {
    pub enabled: bool,
    pub requests_per_second: u32,
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
    pub fn builder() -> SecurityConfigBuilder {
        SecurityConfigBuilder::default()
    }

    pub fn validate(&self) -> Result<()> {
        if self.tls.enabled
            && self.tls.client_cert_path.is_some()
            && self.tls.client_key_path.is_none()
        {
            return Err(crate::error::A2aError::Tls(
                "Client certificate provided but client key is missing".to_string(),
            ));
        }

        if let Some(cert_path) = &self.tls.client_cert_path {
            if !Path::new(cert_path).exists() {
                return Err(crate::error::A2aError::Tls(format!(
                    "Client certificate file not found: {cert_path}"
                )));
            }
        }

        if let Some(key_path) = &self.tls.client_key_path {
            if !Path::new(key_path).exists() {
                return Err(crate::error::A2aError::Tls(format!(
                    "Client key file not found: {key_path}"
                )));
            }
        }

        Ok(())
    }

    pub fn load_certificates(&self) -> Result<CertificateData> {
        let mut data = CertificateData::default();

        if let Some(cert_path) = &self.tls.client_cert_path {
            debug!("Loading client certificate from: {}", cert_path);
            data.client_cert = Some(fs::read_to_string(cert_path).map_err(|e| {
                crate::error::A2aError::Tls(format!("Failed to read client cert: {e}"))
            })?);
        }

        if let Some(key_path) = &self.tls.client_key_path {
            debug!("Loading client key from: {}", key_path);
            data.client_key = Some(fs::read_to_string(key_path).map_err(|e| {
                crate::error::A2aError::Tls(format!("Failed to read client key: {e}"))
            })?);
        }

        if let Some(ca_path) = &self.tls.ca_cert_path {
            debug!("Loading CA certificate from: {}", ca_path);
            data.ca_cert = Some(fs::read_to_string(ca_path).map_err(|e| {
                crate::error::A2aError::Tls(format!("Failed to read CA cert: {e}"))
            })?);
        }

        Ok(data)
    }
}

#[derive(Default)]
pub struct CertificateData {
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub ca_cert: Option<String>,
}

#[derive(Default)]
pub struct SecurityConfigBuilder {
    config: SecurityConfig,
}

impl SecurityConfigBuilder {
    pub fn tls_enabled(mut self, enabled: bool) -> Self {
        self.config.tls.enabled = enabled;
        self
    }

    pub fn verify_certificates(mut self, verify: bool) -> Self {
        self.config.tls.verify_certificates = verify;
        self
    }

    pub fn minimum_tls_version(mut self, version: TlsVersion) -> Self {
        self.config.tls.minimum_tls_version = version;
        self
    }

    pub fn client_certificates(mut self, cert_path: String, key_path: String) -> Self {
        self.config.tls.client_cert_path = Some(cert_path);
        self.config.tls.client_key_path = Some(key_path);
        self
    }

    pub fn ca_certificate(mut self, ca_path: String) -> Self {
        self.config.tls.ca_cert_path = Some(ca_path);
        self
    }

    pub fn auth_method(mut self, method: AuthMethod) -> Self {
        self.config.authentication.method = method;
        self
    }

    pub fn bearer_token(mut self, token: String) -> Self {
        self.config.authentication.method = AuthMethod::Bearer;
        self.config.authentication.token = Some(token);
        self
    }

    pub fn api_key(mut self, key: String) -> Self {
        self.config.authentication.method = AuthMethod::ApiKey;
        self.config.authentication.api_key = Some(key);
        self
    }

    pub fn rate_limiting(mut self, enabled: bool, rps: u32, burst: u32) -> Self {
        self.config.rate_limiting = RateLimitSettings {
            enabled,
            requests_per_second: rps,
            burst_size: burst,
        };
        self
    }

    pub fn build(self) -> Result<SecurityConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

#[cfg(test)]
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
