//! TDF Manifest structures
//!
//! This module contains the data structures for TDF manifests, including:
//! - Payload information
//! - Encryption configuration
//! - Key access objects
//! - Integrity information (segments and root signature)
//!
//! Note: Cryptographic operations (HMAC, policy binding generation) are in the crypto crate.

use serde::{Deserialize, Serialize};

/// TDF manifest structure
#[derive(Debug, Serialize, Deserialize)]
pub struct TdfManifest {
    pub payload: Payload,
    #[serde(rename = "encryptionInformation")]
    pub encryption_information: EncryptionInformation,
    #[serde(rename = "schemaVersion", skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
}

/// Payload reference in TDF manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
    #[serde(rename = "type")]
    pub payload_type: String,
    pub url: String,
    pub protocol: String,
    #[serde(rename = "isEncrypted")]
    pub is_encrypted: bool,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(rename = "tdf_spec_version", skip_serializing_if = "Option::is_none")]
    pub tdf_spec_version: Option<String>,
}

impl Default for Payload {
    fn default() -> Self {
        Self {
            payload_type: "reference".to_string(),
            url: "0.payload".to_string(),
            protocol: "zip".to_string(),
            is_encrypted: true,
            mime_type: Some("application/octet-stream".to_string()),
            tdf_spec_version: Some("3.0.0".to_string()),
        }
    }
}

/// Encryption information in TDF manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionInformation {
    #[serde(rename = "type")]
    pub encryption_type: String,
    #[serde(rename = "keyAccess")]
    pub key_access: Vec<KeyAccess>,
    pub method: EncryptionMethod,
    #[serde(rename = "integrityInformation")]
    pub integrity_information: IntegrityInformation,
    pub policy: String,
}

/// Policy binding structure
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PolicyBinding {
    pub alg: String,
    pub hash: String,
}

/// Key access type constants for TDF3
pub mod key_access_type {
    /// Standard RSA-wrapped key (RSA-OAEP)
    pub const WRAPPED: &str = "wrapped";
    /// EC-wrapped key (ECIES: ECDH + HKDF + AES-GCM)
    pub const EC_WRAPPED: &str = "ec-wrapped";
    /// Remote key access (key stored on KAS)
    pub const REMOTE: &str = "remote";
    /// Remote wrapped key access
    pub const REMOTE_WRAPPED: &str = "remoteWrapped";
}

/// Key access object in manifest
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyAccess {
    #[serde(rename = "type")]
    pub access_type: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
    pub protocol: String,
    #[serde(rename = "wrappedKey")]
    pub wrapped_key: String,
    #[serde(rename = "policyBinding")]
    pub policy_binding: PolicyBinding,
    #[serde(rename = "encryptedMetadata", skip_serializing_if = "Option::is_none")]
    pub encrypted_metadata: Option<String>,
    #[serde(rename = "schemaVersion", skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    /// Ephemeral public key for EC-wrapped keys (PEM format)
    #[serde(rename = "ephemeralPublicKey", skip_serializing_if = "Option::is_none")]
    pub ephemeral_public_key: Option<String>,
}

impl KeyAccess {
    /// Creates a new KeyAccess object with default values (RSA-wrapped)
    pub fn new(url: String) -> Self {
        KeyAccess {
            access_type: key_access_type::WRAPPED.to_string(),
            url,
            kid: None,
            protocol: "kas".to_string(),
            wrapped_key: String::new(),
            policy_binding: PolicyBinding {
                alg: "HS256".to_string(),
                hash: String::new(),
            },
            encrypted_metadata: None,
            schema_version: Some("1.0".to_string()),
            ephemeral_public_key: None,
        }
    }

    /// Creates a new KeyAccess object for EC-wrapped keys
    pub fn new_ec_wrapped(url: String) -> Self {
        KeyAccess {
            access_type: key_access_type::EC_WRAPPED.to_string(),
            url,
            kid: None,
            protocol: "kas".to_string(),
            wrapped_key: String::new(),
            policy_binding: PolicyBinding {
                alg: "HS256".to_string(),
                hash: String::new(),
            },
            encrypted_metadata: None,
            schema_version: Some("1.0".to_string()),
            ephemeral_public_key: None,
        }
    }

    /// Check if this key access uses EC wrapping
    pub fn is_ec_wrapped(&self) -> bool {
        self.access_type == key_access_type::EC_WRAPPED
    }

    /// Check if this key access uses RSA wrapping
    pub fn is_rsa_wrapped(&self) -> bool {
        self.access_type == key_access_type::WRAPPED
    }

    /// Set the ephemeral public key (for EC-wrapped keys)
    pub fn set_ephemeral_public_key(&mut self, pem: String) {
        self.ephemeral_public_key = Some(pem);
    }
}

/// Encryption method configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionMethod {
    pub algorithm: String,
    #[serde(rename = "isStreamable")]
    pub is_streamable: bool,
    pub iv: String,
}

impl Default for EncryptionMethod {
    fn default() -> Self {
        Self {
            algorithm: "AES-256-GCM".to_string(),
            is_streamable: true,
            iv: String::new(),
        }
    }
}

/// Integrity information including segments and root signature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityInformation {
    #[serde(rename = "rootSignature")]
    pub root_signature: RootSignature,
    #[serde(rename = "segmentHashAlg")]
    pub segment_hash_alg: String,
    pub segments: Vec<Segment>,
    #[serde(rename = "segmentSizeDefault")]
    pub segment_size_default: u64,
    #[serde(rename = "encryptedSegmentSizeDefault")]
    pub encrypted_segment_size_default: u64,
}

impl Default for IntegrityInformation {
    fn default() -> Self {
        Self {
            root_signature: RootSignature::default(),
            segment_hash_alg: "GMAC".to_string(),
            segments: Vec::new(),
            segment_size_default: 1024 * 1024,                // 1MB
            encrypted_segment_size_default: 1024 * 1024 + 28, // +IV+tag
        }
    }
}

/// Root signature for integrity verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootSignature {
    pub alg: String,
    pub sig: String,
}

impl Default for RootSignature {
    fn default() -> Self {
        Self {
            alg: "HS256".to_string(),
            sig: String::new(),
        }
    }
}

/// Segment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub hash: String,
    #[serde(rename = "segmentSize", skip_serializing_if = "Option::is_none")]
    pub segment_size: Option<u64>,
    #[serde(
        rename = "encryptedSegmentSize",
        skip_serializing_if = "Option::is_none"
    )]
    pub encrypted_segment_size: Option<u64>,
}

impl TdfManifest {
    /// Creates a new TDF manifest with basic structure
    pub fn new(payload_url: String, kas_url: String) -> Self {
        TdfManifest {
            payload: Payload {
                payload_type: "reference".to_string(),
                url: payload_url,
                protocol: "zip".to_string(),
                is_encrypted: true,
                mime_type: Some("application/octet-stream".to_string()),
                tdf_spec_version: Some("3.0.0".to_string()),
            },
            encryption_information: EncryptionInformation {
                encryption_type: "split".to_string(),
                key_access: vec![KeyAccess::new(kas_url)],
                method: EncryptionMethod {
                    algorithm: "AES-256-GCM".to_string(),
                    is_streamable: true,
                    iv: String::new(),
                },
                integrity_information: IntegrityInformation {
                    root_signature: RootSignature {
                        alg: "HS256".to_string(),
                        sig: String::new(),
                    },
                    segment_hash_alg: "GMAC".to_string(),
                    segments: Vec::new(),
                    segment_size_default: 1024 * 1024, // 1MB default
                    encrypted_segment_size_default: 1024 * 1024 + 28, // +IV+tag
                },
                policy: String::new(),
            },
            schema_version: Some("3.0.0".to_string()),
        }
    }

    /// Set the policy for the manifest using a raw string
    pub fn set_policy_raw(&mut self, policy: &str) {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
        self.encryption_information.policy = BASE64.encode(policy);
    }

    /// Get the decoded policy from the manifest as a raw string
    pub fn get_policy_raw(&self) -> Result<String, base64::DecodeError> {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
        let bytes = BASE64.decode(&self.encryption_information.policy)?;
        String::from_utf8(bytes)
            .map_err(|err| base64::DecodeError::InvalidByte(err.utf8_error().valid_up_to(), 0))
    }

    /// Add a segment to the manifest
    pub fn add_segment(
        &mut self,
        hash: String,
        segment_size: Option<u64>,
        encrypted_segment_size: Option<u64>,
    ) {
        self.encryption_information
            .integrity_information
            .segments
            .push(Segment {
                hash,
                segment_size,
                encrypted_segment_size,
            });
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize from JSON
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

impl KeyAccess {
    /// Set encrypted metadata
    pub fn set_encrypted_metadata(&mut self, metadata: &str) {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
        self.encrypted_metadata = Some(BASE64.encode(metadata));
    }

    /// Clear encrypted metadata
    pub fn clear_encrypted_metadata(&mut self) {
        self.encrypted_metadata = None;
    }
}
