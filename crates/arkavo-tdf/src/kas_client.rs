//! Arkavo KAS (Key Access Service) client implementation.
//!
//! Provides OAuth token acquisition from identity.arkavo.net and
//! key rewrap operations via the KAS at 100.arkavo.net.
//!
//! Enable with the `kas` feature flag.

use async_trait::async_trait;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::error::TdfError;
use crate::traits::KasClient;
use crate::types::TdfManifest;

/// Default Arkavo OAuth endpoint.
pub const ARKAVO_OAUTH_URL: &str = "https://identity.arkavo.net";

/// Default Arkavo KAS endpoint.
pub const ARKAVO_KAS_URL: &str = "https://100.arkavo.net";

/// KAS public key endpoint path.
pub const KAS_PUBLIC_KEY_PATH: &str = "/kas/v2/kas_public_key";

/// Configuration for Arkavo KAS client.
#[derive(Debug, Clone)]
pub struct ArkavoKasConfig {
    /// OAuth identity provider URL (default: identity.arkavo.net)
    pub oauth_url: String,
    /// KAS service URL (default: 100.arkavo.net)
    pub kas_url: String,
    /// OAuth client ID
    pub client_id: String,
    /// OAuth client secret (optional for public clients)
    pub client_secret: Option<String>,
    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl ArkavoKasConfig {
    /// Create config with client credentials.
    #[must_use]
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            oauth_url: ARKAVO_OAUTH_URL.to_string(),
            kas_url: ARKAVO_KAS_URL.to_string(),
            client_id: client_id.into(),
            client_secret: None,
            timeout_secs: 30,
        }
    }

    /// Set client secret for confidential clients.
    #[must_use]
    pub fn with_client_secret(mut self, secret: impl Into<String>) -> Self {
        self.client_secret = Some(secret.into());
        self
    }

    /// Override OAuth URL.
    #[must_use]
    pub fn with_oauth_url(mut self, url: impl Into<String>) -> Self {
        self.oauth_url = url.into();
        self
    }

    /// Override KAS URL.
    #[must_use]
    pub fn with_kas_url(mut self, url: impl Into<String>) -> Self {
        self.kas_url = url.into();
        self
    }

    /// Set request timeout.
    #[must_use]
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

impl Default for ArkavoKasConfig {
    fn default() -> Self {
        Self {
            oauth_url: ARKAVO_OAUTH_URL.to_string(),
            kas_url: ARKAVO_KAS_URL.to_string(),
            client_id: String::new(),
            client_secret: None,
            timeout_secs: 30,
        }
    }
}

/// OAuth token response from identity provider.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    expires_in: Option<u64>,
    #[serde(default)]
    #[allow(dead_code)]
    scope: String,
}

/// KAS public key response.
#[derive(Debug, Deserialize)]
pub struct KasPublicKey {
    /// Public key in PEM format
    #[serde(rename = "publicKey")]
    pub public_key: String,
    /// Key ID
    #[serde(rename = "kid")]
    pub key_id: Option<String>,
    /// Algorithm (e.g., "RSA-OAEP")
    #[serde(rename = "algorithm")]
    pub algorithm: Option<String>,
}

/// Arkavo KAS client with OAuth integration.
///
/// Handles:
/// 1. OAuth token acquisition from identity.arkavo.net
/// 2. Key rewrap operations via KAS at 100.arkavo.net
/// 3. Token refresh on expiration
#[cfg(feature = "kas")]
pub struct ArkavoKasClient {
    config: ArkavoKasConfig,
    http_client: reqwest::Client,
    /// Cached OAuth token
    access_token: std::sync::RwLock<Option<CachedToken>>,
}

#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    expires_at: std::time::Instant,
}

#[cfg(feature = "kas")]
impl ArkavoKasClient {
    /// Create a new Arkavo KAS client.
    pub fn new(config: ArkavoKasConfig) -> Result<Self, TdfError> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| TdfError::Kas(format!("Failed to create HTTP client: {e}")))?;

        Ok(Self {
            config,
            http_client,
            access_token: std::sync::RwLock::new(None),
        })
    }

    /// Create with default Arkavo endpoints.
    pub fn with_client_id(client_id: impl Into<String>) -> Result<Self, TdfError> {
        Self::new(ArkavoKasConfig::new(client_id))
    }

    /// Acquire OAuth token from identity provider.
    ///
    /// Uses client credentials flow to get an access token.
    /// Tokens are cached and reused until near expiration.
    #[allow(clippy::missing_panics_doc)] // RwLock poisoning is unrecoverable
    pub async fn acquire_token(&self) -> Result<String, TdfError> {
        // Check cache first
        {
            let cache = self.access_token.read().unwrap();
            if let Some(cached) = cache.as_ref()
                && cached.expires_at > std::time::Instant::now()
            {
                debug!("Using cached OAuth token");
                return Ok(cached.token.clone());
            }
        }

        info!("Acquiring OAuth token from {}", self.config.oauth_url);

        // Build token request
        let token_url = format!("{}/oauth/token", self.config.oauth_url);

        let mut params = vec![
            ("grant_type", "client_credentials"),
            ("client_id", &self.config.client_id),
        ];

        let secret_str;
        if let Some(secret) = &self.config.client_secret {
            secret_str = secret.clone();
            params.push(("client_secret", &secret_str));
        }

        let response = self
            .http_client
            .post(&token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| TdfError::Kas(format!("OAuth token request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(TdfError::Kas(format!(
                "OAuth token request failed: HTTP {status}: {body}"
            )));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| TdfError::Kas(format!("Failed to parse token response: {e}")))?;

        // Cache the token
        let expires_in = token_response.expires_in.unwrap_or(3600);
        let expires_at = std::time::Instant::now()
            + std::time::Duration::from_secs(expires_in.saturating_sub(60));

        {
            let mut cache = self.access_token.write().unwrap();
            *cache = Some(CachedToken {
                token: token_response.access_token.clone(),
                expires_at,
            });
        }

        info!("OAuth token acquired, expires in {}s", expires_in);
        Ok(token_response.access_token)
    }

    /// Get the KAS public key.
    pub async fn get_kas_public_key(&self) -> Result<KasPublicKey, TdfError> {
        let url = format!("{}{}", self.config.kas_url, KAS_PUBLIC_KEY_PATH);

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| TdfError::Kas(format!("KAS public key request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(TdfError::Kas(format!(
                "KAS public key request failed: HTTP {status}: {body}"
            )));
        }

        response
            .json()
            .await
            .map_err(|e| TdfError::Kas(format!("Failed to parse KAS public key: {e}")))
    }

    /// Rewrap key from TDF manifest.
    ///
    /// This performs the complete KAS rewrap flow:
    /// 1. Acquire OAuth token (cached)
    /// 2. Create opentdf KAS client
    /// 3. Perform rewrap operation
    pub async fn rewrap_manifest(&self, manifest: &TdfManifest) -> Result<Vec<u8>, TdfError> {
        // Get OAuth token
        let token = self.acquire_token().await?;

        // Convert arkavo-tdf manifest to opentdf manifest
        let opentdf_manifest = self.to_opentdf_manifest(manifest)?;

        // Create or get KAS client
        let kas_client = opentdf::kas::KasClient::new(&self.config.kas_url, &token)
            .map_err(|e| TdfError::Kas(format!("Failed to create KAS client: {e}")))?;

        // Perform rewrap
        kas_client
            .rewrap_standard_tdf(&opentdf_manifest)
            .await
            .map_err(|e| TdfError::Kas(format!("Rewrap failed: {e}")))
    }

    /// Convert arkavo-tdf manifest to opentdf manifest.
    fn to_opentdf_manifest(
        &self,
        manifest: &TdfManifest,
    ) -> Result<opentdf::TdfManifest, TdfError> {
        // Build key access objects
        let key_access: Vec<opentdf::KeyAccess> = manifest
            .encryption_information
            .key_access
            .iter()
            .map(|ka| opentdf::KeyAccess {
                access_type: ka.access_type.clone(),
                url: ka.url.clone(),
                kid: None,
                protocol: ka.protocol.clone(),
                wrapped_key: ka.wrapped_key.clone(),
                policy_binding: opentdf::PolicyBinding {
                    alg: ka.policy_binding.alg.clone(),
                    hash: ka.policy_binding.hash.clone(),
                },
                encrypted_metadata: None,
                schema_version: Some("1.0".to_string()),
            })
            .collect();

        // Build encryption information
        let encryption_info = opentdf::EncryptionInformation {
            encryption_type: manifest.encryption_information.key_type.clone(),
            key_access,
            method: opentdf::EncryptionMethod {
                algorithm: manifest.encryption_information.method.algorithm.clone(),
                is_streamable: manifest.encryption_information.method.is_streamable,
                iv: manifest.encryption_information.method.iv.clone(),
            },
            integrity_information: opentdf::IntegrityInformation {
                root_signature: opentdf::RootSignature {
                    alg: "HS256".to_string(),
                    sig: String::new(),
                },
                segment_hash_alg: "GMAC".to_string(),
                segments: vec![],
                segment_size_default: 0,
                encrypted_segment_size_default: 0,
            },
            policy: manifest.encryption_information.policy.clone(),
        };

        // Build payload
        let payload = opentdf::Payload {
            payload_type: manifest.payload.payload_type.clone(),
            url: "inline".to_string(),
            protocol: manifest.payload.protocol.clone(),
            is_encrypted: true,
            mime_type: Some(manifest.payload.mime_type.clone()),
            tdf_spec_version: None,
        };

        Ok(opentdf::TdfManifest {
            payload,
            encryption_information: encryption_info,
            schema_version: Some("1.1.0".to_string()),
        })
    }

    /// Decrypt a TDF manifest using KAS.
    ///
    /// Performs:
    /// 1. Rewrap to get payload key
    /// 2. Decrypt the payload using the key
    pub async fn decrypt_manifest(&self, manifest: &TdfManifest) -> Result<Vec<u8>, TdfError> {
        // Get the payload key via rewrap
        let payload_key = self.rewrap_manifest(manifest).await?;

        // Convert to opentdf format for decryption
        let opentdf_manifest = opentdf::jsonrpc::TdfManifestInline {
            payload: opentdf::jsonrpc::InlinePayload {
                payload_type: manifest.payload.payload_type.clone(),
                mime_type: manifest.payload.mime_type.clone(),
                protocol: manifest.payload.protocol.clone(),
                value: manifest.payload.value.clone(),
                is_encrypted: true,
            },
            encryption_information: opentdf::EncryptionInformation {
                encryption_type: manifest.encryption_information.key_type.clone(),
                key_access: manifest
                    .encryption_information
                    .key_access
                    .iter()
                    .map(|ka| opentdf::KeyAccess {
                        access_type: ka.access_type.clone(),
                        url: ka.url.clone(),
                        kid: None,
                        protocol: ka.protocol.clone(),
                        wrapped_key: ka.wrapped_key.clone(),
                        policy_binding: opentdf::PolicyBinding {
                            alg: ka.policy_binding.alg.clone(),
                            hash: ka.policy_binding.hash.clone(),
                        },
                        encrypted_metadata: None,
                        schema_version: Some("1.0".to_string()),
                    })
                    .collect(),
                method: opentdf::EncryptionMethod {
                    algorithm: manifest.encryption_information.method.algorithm.clone(),
                    is_streamable: manifest.encryption_information.method.is_streamable,
                    iv: manifest.encryption_information.method.iv.clone(),
                },
                integrity_information: opentdf::IntegrityInformation {
                    root_signature: opentdf::RootSignature {
                        alg: "HS256".to_string(),
                        sig: String::new(),
                    },
                    segment_hash_alg: "GMAC".to_string(),
                    segments: vec![],
                    segment_size_default: 0,
                    encrypted_segment_size_default: 0,
                },
                policy: manifest.encryption_information.policy.clone(),
            },
            schema_version: Some("1.1.0".to_string()),
        };

        let envelope = opentdf::jsonrpc::TdfJsonRpc {
            manifest: opentdf_manifest,
            version: manifest.version.clone(),
        };

        // Decrypt using the payload key
        envelope
            .decrypt_with_key(&payload_key)
            .map_err(|e| TdfError::Decryption(format!("Decryption failed: {e}")))
    }
}

#[cfg(feature = "kas")]
#[async_trait]
impl KasClient for ArkavoKasClient {
    async fn rewrap(&self, wrapped_key: &str, policy: &str) -> Result<Vec<u8>, TdfError> {
        // This is a simplified interface - for full manifest rewrap use rewrap_manifest
        warn!(
            "Using simplified rewrap interface - consider using rewrap_manifest for full TDF support"
        );

        // Build a minimal manifest for rewrap
        let manifest = TdfManifest {
            encryption_information: crate::types::EncryptionInformation {
                key_type: "split".to_string(),
                key_access: vec![crate::types::KeyAccessObject {
                    access_type: "wrapped".to_string(),
                    url: self.config.kas_url.clone(),
                    protocol: "kas".to_string(),
                    wrapped_key: wrapped_key.to_string(),
                    policy_binding: crate::types::PolicyBinding {
                        alg: "HS256".to_string(),
                        hash: String::new(),
                    },
                }],
                method: crate::types::EncryptionMethod::default(),
                policy: policy.to_string(),
            },
            payload: crate::types::InlinePayload::default(),
            version: "3.0.0".to_string(),
        };

        self.rewrap_manifest(&manifest).await
    }

    async fn health_check(&self) -> Result<bool, TdfError> {
        match self.get_kas_public_key().await {
            Ok(_) => Ok(true),
            Err(e) => {
                warn!("KAS health check failed: {e}");
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_builder() {
        let config = ArkavoKasConfig::new("test-client")
            .with_client_secret("secret")
            .with_timeout(60);

        assert_eq!(config.client_id, "test-client");
        assert_eq!(config.client_secret, Some("secret".to_string()));
        assert_eq!(config.timeout_secs, 60);
        assert_eq!(config.oauth_url, ARKAVO_OAUTH_URL);
        assert_eq!(config.kas_url, ARKAVO_KAS_URL);
    }

    #[test]
    fn config_custom_urls() {
        let config = ArkavoKasConfig::new("client")
            .with_oauth_url("https://custom-oauth.example.com")
            .with_kas_url("https://custom-kas.example.com");

        assert_eq!(config.oauth_url, "https://custom-oauth.example.com");
        assert_eq!(config.kas_url, "https://custom-kas.example.com");
    }

    #[test]
    fn default_config() {
        let config = ArkavoKasConfig::default();
        assert_eq!(config.oauth_url, ARKAVO_OAUTH_URL);
        assert_eq!(config.kas_url, ARKAVO_KAS_URL);
        assert!(config.client_id.is_empty());
    }
}
