//! Real TDF encryption/decryption using opentdf-rs.
//!
//! Enable with the `opentdf` feature flag.

// Allow deprecated opentdf types until migration to TdfJson API.
// The deprecated API (TdfJsonRpc, TdfManifestInline, InlinePayload) will be removed in opentdf 1.0.0.
// TODO: Migrate to TdfJson API which requires KAS public key for EC key wrapping.
#![allow(deprecated)]

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::TdfError;
use crate::traits::{TdfDecryptor, TdfEncryptor};
use crate::types::{
    EncryptionInformation, EncryptionMethod, InlinePayload, KeyAccessObject, Policy, PolicyBinding,
    TdfManifest,
};

/// Configuration for OpenTDF service.
#[derive(Debug, Clone)]
pub struct OpenTdfConfig {
    /// KAS URL for key access
    pub kas_url: String,
    /// Segment size for large file encryption (default: 2MB)
    pub segment_size: usize,
}

impl OpenTdfConfig {
    /// Create config with KAS URL.
    #[must_use]
    pub fn new(kas_url: impl Into<String>) -> Self {
        Self {
            kas_url: kas_url.into(),
            segment_size: 2 * 1024 * 1024, // 2MB default
        }
    }

    /// Set segment size for large file encryption.
    #[must_use]
    pub fn with_segment_size(mut self, size: usize) -> Self {
        self.segment_size = size;
        self
    }
}

impl Default for OpenTdfConfig {
    fn default() -> Self {
        Self {
            kas_url: "https://kas.arkavo.net".to_string(),
            segment_size: 2 * 1024 * 1024,
        }
    }
}

/// TDF service implementation using opentdf-rs.
///
/// Provides AES-256-GCM encryption with ZTDF-JSON format.
#[derive(Debug, Clone)]
pub struct OpenTdfService {
    config: OpenTdfConfig,
}

impl OpenTdfService {
    /// Create a new OpenTDF service with configuration.
    #[must_use]
    pub fn new(config: OpenTdfConfig) -> Self {
        Self { config }
    }

    /// Create with KAS URL using defaults.
    #[must_use]
    pub fn with_kas_url(kas_url: impl Into<String>) -> Self {
        Self::new(OpenTdfConfig::new(kas_url))
    }

    /// Convert arkavo-tdf Policy to opentdf Policy.
    fn to_opentdf_policy(policy: &Policy) -> opentdf::Policy {
        // Convert attributes to opentdf AttributePolicy conditions
        let body: Vec<opentdf::AttributePolicy> = policy
            .attributes
            .iter()
            .filter_map(|attr| {
                // Parse FQN to get namespace and name
                opentdf::fqn::AttributeFqn::parse(&attr.attribute)
                    .ok()
                    .map(|fqn| {
                        opentdf::AttributePolicy::condition(
                            fqn.to_identifier(),
                            opentdf::Operator::In,
                            opentdf::AttributeValue::StringArray(attr.values.clone()),
                        )
                    })
            })
            .collect();

        opentdf::Policy::new(
            policy
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            body,
            policy.dissemination.clone(),
        )
    }
}

impl Default for OpenTdfService {
    fn default() -> Self {
        Self::new(OpenTdfConfig::default())
    }
}

#[async_trait]
impl TdfEncryptor for OpenTdfService {
    async fn encrypt(&self, plaintext: &[u8], policy: &Policy) -> Result<TdfManifest, TdfError> {
        let opentdf_policy = Self::to_opentdf_policy(policy);

        // Use ZTDF-JSON format for inline payloads
        let envelope = opentdf::jsonrpc::TdfJsonRpc::encrypt(plaintext)
            .kas_url(&self.config.kas_url)
            .policy(opentdf_policy)
            .build()
            .map_err(|e| TdfError::Encryption(format!("opentdf encryption failed: {e}")))?;

        // Convert to arkavo-tdf format
        Ok(TdfManifest {
            encryption_information: EncryptionInformation {
                key_type: envelope
                    .manifest
                    .encryption_information
                    .encryption_type
                    .clone(),
                key_access: envelope
                    .manifest
                    .encryption_information
                    .key_access
                    .iter()
                    .map(|ka| KeyAccessObject {
                        access_type: ka.access_type.clone(),
                        url: ka.url.clone(),
                        protocol: ka.protocol.clone(),
                        wrapped_key: ka.wrapped_key.clone(),
                        policy_binding: PolicyBinding {
                            alg: ka.policy_binding.alg.clone(),
                            hash: ka.policy_binding.hash.clone(),
                        },
                    })
                    .collect(),
                method: EncryptionMethod {
                    algorithm: envelope
                        .manifest
                        .encryption_information
                        .method
                        .algorithm
                        .clone(),
                    iv: envelope.manifest.encryption_information.method.iv.clone(),
                    is_streamable: envelope
                        .manifest
                        .encryption_information
                        .method
                        .is_streamable,
                },
                policy: envelope.manifest.encryption_information.policy.clone(),
            },
            payload: InlinePayload {
                payload_type: envelope.manifest.payload.payload_type.clone(),
                mime_type: envelope.manifest.payload.mime_type.clone(),
                protocol: envelope.manifest.payload.protocol.clone(),
                value: envelope.manifest.payload.value.clone(),
            },
            version: envelope.version.clone(),
        })
    }

    async fn encrypt_stream<R>(
        &self,
        mut reader: R,
        policy: &Policy,
    ) -> Result<TdfManifest, TdfError>
    where
        R: AsyncRead + Send + Unpin,
    {
        // Read entire stream into memory for encryption
        // For truly streaming encryption, would need opentdf streaming support
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).await?;
        self.encrypt(&buffer, policy).await
    }
}

#[async_trait]
impl TdfDecryptor for OpenTdfService {
    async fn decrypt(&self, manifest: &TdfManifest) -> Result<Vec<u8>, TdfError> {
        // Create opentdf TdfJsonRpc from manifest
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
                        ephemeral_public_key: None,
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

        let _envelope = opentdf::jsonrpc::TdfJsonRpc {
            manifest: opentdf_manifest,
            version: manifest.version.clone(),
        };

        // Decryption requires payload key from KAS
        // The envelope would be used with KasClient to get the key
        Err(TdfError::Decryption(
            "Decryption requires KAS client - use decrypt_with_kas".to_string(),
        ))
    }

    async fn decrypt_stream<W>(
        &self,
        manifest: &TdfManifest,
        mut writer: W,
    ) -> Result<u64, TdfError>
    where
        W: AsyncWrite + Send + Unpin,
    {
        let plaintext = self.decrypt(manifest).await?;
        writer.write_all(&plaintext).await?;
        Ok(plaintext.len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PolicyBuilder;
    use arkavo_test_macros::spec;
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

    #[spec("TDF-001")]
    #[tokio::test]
    async fn tdf_001_encrypt_data_with_policy() {
        let service = OpenTdfService::with_kas_url("https://kas.example.com");
        let policy = PolicyBuilder::new()
            .attribute("https://arkavo.net/attr/role", &["admin"])
            .build()
            .unwrap();
        let plaintext = b"TDF-001 secret payload";

        let manifest = service.encrypt(plaintext, &policy).await.unwrap();

        assert_eq!(
            manifest.encryption_information.method.algorithm,
            "AES-256-GCM"
        );
        assert!(!manifest.encryption_information.key_access.is_empty());

        let key_access = &manifest.encryption_information.key_access[0];
        assert_eq!(key_access.access_type, "wrapped");
        assert_eq!(key_access.protocol, "kas");
        assert!(!key_access.wrapped_key.is_empty());
        assert!(!key_access.policy_binding.hash.is_empty());
        assert_eq!(key_access.policy_binding.alg, "HS256");

        assert_eq!(manifest.payload.payload_type, "inline");
        assert!(!manifest.payload.value.is_empty());
        let ciphertext = BASE64
            .decode(&manifest.payload.value)
            .expect("payload is base64");
        assert_ne!(ciphertext, plaintext.to_vec());

        let policy_json = BASE64
            .decode(&manifest.encryption_information.policy)
            .expect("policy is base64");
        let embedded_policy: serde_json::Value =
            serde_json::from_slice(&policy_json).expect("policy is JSON");
        let policy_str = embedded_policy.to_string();
        assert!(policy_str.contains("arkavo.net"));
        assert!(policy_str.contains("role"));
        assert!(policy_str.contains("admin"));
    }

    #[spec("TDFS-001")]
    #[tokio::test]
    async fn encrypt_basic() {
        let service = OpenTdfService::with_kas_url("https://kas.example.com");
        let policy = PolicyBuilder::new()
            .attribute("https://test.com/attr/level", &["secret"])
            .build()
            .unwrap();

        let plaintext = b"Hello, OpenTDF!";
        let manifest = service.encrypt(plaintext, &policy).await.unwrap();

        assert_eq!(manifest.version, "3.0.0");
        assert_eq!(manifest.payload.payload_type, "inline");
        assert_eq!(
            manifest.encryption_information.method.algorithm,
            "AES-256-GCM"
        );
        assert!(!manifest.payload.value.is_empty());
    }

    #[spec("TDFS-001")]
    #[tokio::test]
    async fn encrypt_with_dissemination() {
        let service = OpenTdfService::with_kas_url("https://kas.example.com");
        let policy = PolicyBuilder::new()
            .attribute("https://arkavo.net/attr/access", &["allowed"])
            .dissemination(&["user@example.com", "admin@example.com"])
            .build()
            .unwrap();

        let manifest = service.encrypt(b"Sensitive data", &policy).await.unwrap();

        assert_eq!(manifest.encryption_information.key_type, "split");
        assert!(!manifest.encryption_information.key_access.is_empty());
    }

    #[spec("TDFS-001")]
    #[tokio::test]
    async fn encrypt_stream() {
        use std::io::Cursor;

        let service = OpenTdfService::with_kas_url("https://kas.example.com");
        let policy = PolicyBuilder::new()
            .attribute("https://test.com/attr/type", &["document"])
            .build()
            .unwrap();

        let data = b"Streaming data for encryption";
        let reader = Cursor::new(data.to_vec());

        let manifest = service.encrypt_stream(reader, &policy).await.unwrap();

        assert!(!manifest.payload.value.is_empty());
    }

    #[spec("TDFS-003")]
    #[tokio::test]
    async fn manifest_serialization() {
        let service = OpenTdfService::with_kas_url("https://kas.example.com");
        let policy = PolicyBuilder::new()
            .attribute("https://test.com/attr/test", &["value"])
            .build()
            .unwrap();

        let manifest = service.encrypt(b"Test data", &policy).await.unwrap();

        // Serialize and deserialize
        let json = serde_json::to_string(&manifest).unwrap();
        let deserialized: TdfManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(manifest.version, deserialized.version);
        assert_eq!(manifest.payload.value, deserialized.payload.value);
    }
}
