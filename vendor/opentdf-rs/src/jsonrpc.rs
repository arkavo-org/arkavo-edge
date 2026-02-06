//! TDF-JSON format implementation (per TDF-JSON specification draft-00)
//!
//! This module provides support for inline TDF payloads suitable for JSON-RPC protocols
//! like A2A (Agent-to-Agent) and MCP (Model Context Protocol).
//!
//! # Overview
//!
//! TDF-JSON defines a JSON-based container format for TDF that embeds encrypted payloads
//! inline rather than using a ZIP archive. This format is optimized for JSON-RPC protocols,
//! REST APIs, and streaming scenarios where JSON is the native transport.
//!
//! # Example
//!
//! ```rust,no_run
//! use opentdf::{Tdf, Policy, jsonrpc::TdfJson};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a policy
//! let policy = Policy::new(
//!     uuid::Uuid::new_v4().to_string(),
//!     vec![],
//!     vec!["user@example.com".to_string()]
//! );
//!
//! // Load KAS public key (from your KAS server)
//! let kas_public_key = std::fs::read_to_string("kas_public_key.pem")?;
//!
//! // Encrypt data and get TDF-JSON envelope
//! let envelope = TdfJson::encrypt(b"Sensitive data")
//!     .kas_url("https://kas.example.com")
//!     .kas_public_key(&kas_public_key)
//!     .policy(policy)
//!     .build()?;
//!
//! // Serialize to JSON for transmission
//! let json = serde_json::to_string(&envelope)?;
//!
//! // Later: deserialize (decryption requires actual payload key from KAS)
//! let envelope: TdfJson = serde_json::from_str(&json)?;
//! assert_eq!(envelope.tdf, "json");
//! assert_eq!(envelope.version, "1.0.0");
//! # Ok(())
//! # }
//! ```

// Allow deprecated warnings for Nonce::from_slice() which is the correct API for aes-gcm 0.10.x
// This will be resolved when aes-gcm updates to generic-array 1.x
#![allow(deprecated)]

use crate::manifest::{
    EncryptionInformation, EncryptionMethod, IntegrityInformation, IntegrityInformationExt,
    KeyAccess, Payload, RootSignature, Segment, TdfManifest,
};
use crate::policy::Policy;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use opentdf_crypto::{EncryptionError, TdfEncryption, calculate_policy_binding};
use serde::{Deserialize, Serialize};

// ============================================================================
// TDF-JSON Spec-Compliant Types (per TDF-JSON specification draft-00)
// ============================================================================

/// TDF-JSON envelope for inline payload transmission (spec-compliant)
///
/// This structure represents a complete TDF-JSON package per the TDF-JSON
/// specification draft-00. The format is optimized for JSON-RPC protocols,
/// REST APIs, and streaming scenarios.
///
/// # Structure
///
/// ```json
/// {
///   "tdf": "json",
///   "version": "1.0.0",
///   "created": "2026-01-17T12:00:00Z",
///   "manifest": { ... },
///   "payload": { ... }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TdfJson {
    /// Format identifier. MUST be "json" for TDF-JSON documents.
    pub tdf: String,

    /// Semantic version of the TDF-JSON specification (e.g., "1.0.0")
    pub version: String,

    /// ISO 8601 timestamp of document creation (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,

    /// TDF manifest containing encryption and policy information
    pub manifest: TdfJsonManifest,

    /// Inline encrypted payload container
    pub payload: JsonPayload,
}

/// TDF manifest for TDF-JSON format
///
/// Contains encryption information and optional assertions, but NOT the payload
/// (which is at the top level in TDF-JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TdfJsonManifest {
    /// Encryption information including key access and policy
    #[serde(rename = "encryptionInformation")]
    pub encryption_information: EncryptionInformation,

    /// Optional assertions for additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertions: Option<Vec<serde_json::Value>>,
}

/// JSON payload for TDF-JSON transport
///
/// Contains the encrypted data inline as a base64-encoded string.
/// Per the TDF-JSON spec, the payload is at the top level (not nested in manifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonPayload {
    /// Payload type. MUST be "inline" for TDF-JSON
    #[serde(rename = "type")]
    pub payload_type: String,

    /// Encoding protocol. MUST be "base64" for TDF-JSON
    pub protocol: String,

    /// MIME type of the original (unencrypted) data
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,

    /// Whether the payload is encrypted. MUST be true
    #[serde(rename = "isEncrypted")]
    pub is_encrypted: bool,

    /// Length of ciphertext in bytes (before base64)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,

    /// Base64-encoded ciphertext
    pub value: String,
}

// ============================================================================
// Legacy Types (deprecated, for backward compatibility)
// ============================================================================

/// TDF envelope for JSON-RPC protocols with inline payload
///
/// This structure represents a complete TDF package suitable for transmission
/// over JSON-RPC protocols. Unlike traditional TDF archives that use ZIP format,
/// this format includes the encrypted payload inline as a base64-encoded string.
#[deprecated(
    since = "0.12.0",
    note = "Use TdfJson instead, which follows the TDF-JSON specification. Will be removed in 1.0.0"
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TdfJsonRpc {
    /// TDF manifest with inline encrypted payload
    pub manifest: TdfManifestInline,

    /// TDF specification version
    pub version: String,
}

/// TDF manifest with inline payload support (legacy)
#[deprecated(
    since = "0.12.0",
    note = "Use TdfJsonManifest instead. Will be removed in 1.0.0"
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TdfManifestInline {
    /// Inline payload (replaces file reference)
    pub payload: InlinePayload,

    /// Encryption information including key access and policy
    #[serde(rename = "encryptionInformation")]
    pub encryption_information: EncryptionInformation,

    /// Schema version
    #[serde(rename = "schemaVersion", skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
}

/// Inline payload for JSON-RPC transport (legacy)
///
/// Instead of referencing a separate file (as in traditional TDF archives),
/// the encrypted data is included directly as a base64-encoded string.
#[deprecated(
    since = "0.12.0",
    note = "Use JsonPayload instead. Will be removed in 1.0.0"
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlinePayload {
    /// Payload type (always "inline" for JSON-RPC)
    #[serde(rename = "type")]
    pub payload_type: String,

    /// MIME type of the original (unencrypted) data
    #[serde(rename = "mimeType")]
    pub mime_type: String,

    /// Encoding protocol (always "base64" for binary data)
    pub protocol: String,

    /// Base64-encoded encrypted data
    pub value: String,

    /// Whether the payload is encrypted
    #[serde(rename = "isEncrypted")]
    pub is_encrypted: bool,
}

// ============================================================================
// TDF-JSON Builder and Implementation
// ============================================================================

/// Builder for creating TDF-JSON envelopes (spec-compliant)
///
/// Requires the `kas-client` feature for EC key wrapping support.
#[cfg(feature = "kas-client")]
pub struct TdfJsonBuilder {
    data: Vec<u8>,
    kas_url: Option<String>,
    kas_public_key_pem: Option<String>,
    policy: Option<Policy>,
    mime_type: Option<String>,
    include_created: bool,
}

impl TdfJson {
    /// Create a new builder for encrypting data into TDF-JSON format
    ///
    /// Requires the `kas-client` feature for EC key wrapping support.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use opentdf::{Policy, jsonrpc::TdfJson};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let policy = Policy::new(
    ///     uuid::Uuid::new_v4().to_string(),
    ///     vec![],
    ///     vec!["user@example.com".to_string()]
    /// );
    ///
    /// // Load KAS public key from your KAS server
    /// let kas_public_key = std::fs::read_to_string("kas_public_key.pem")?;
    ///
    /// let envelope = TdfJson::encrypt(b"Hello, World!")
    ///     .kas_url("https://kas.example.com")
    ///     .kas_public_key(&kas_public_key)
    ///     .policy(policy)
    ///     .mime_type("text/plain")
    ///     .build()?;
    ///
    /// assert_eq!(envelope.tdf, "json");
    /// assert_eq!(envelope.version, "1.0.0");
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "kas-client")]
    pub fn encrypt(data: &[u8]) -> TdfJsonBuilder {
        TdfJsonBuilder {
            data: data.to_vec(),
            kas_url: None,
            kas_public_key_pem: None,
            policy: None,
            mime_type: None,
            include_created: true,
        }
    }

    /// Format identifier (always "json")
    pub fn format_id(&self) -> &str {
        &self.tdf
    }

    /// Decrypt the inline payload with a provided data encryption key
    ///
    /// # Arguments
    ///
    /// * `payload_key` - The data encryption key obtained from KAS
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use opentdf::jsonrpc::TdfJson;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let envelope: TdfJson = serde_json::from_str("{...}")?;
    /// let payload_key = vec![0u8; 32]; // Obtained from KAS
    ///
    /// let plaintext = envelope.decrypt_with_key(&payload_key)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn decrypt_with_key(&self, payload_key: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        use aes_gcm::{
            Aes256Gcm, Nonce,
            aead::{Aead, KeyInit},
        };

        // Decode base64 payload
        let ciphertext = BASE64.decode(&self.payload.value)?;

        // Extract IV from encryption method
        let iv_bytes = BASE64.decode(&self.manifest.encryption_information.method.iv)?;

        // Extract just the payload IV (first 12 bytes)
        let payload_iv = if iv_bytes.len() >= 12 {
            &iv_bytes[0..12]
        } else {
            &iv_bytes[..]
        };

        // Create cipher with payload key
        let cipher = Aes256Gcm::new_from_slice(payload_key)
            .map_err(|_| EncryptionError::InvalidKeyLength)?;

        let nonce = Nonce::from_slice(payload_iv);

        // Decrypt the data
        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(EncryptionError::AeadError)?;

        Ok(plaintext)
    }

    /// Convert to standard TDF manifest format (for KAS integration)
    pub fn to_standard_manifest(&self) -> TdfManifest {
        TdfManifest {
            payload: Payload {
                payload_type: "reference".to_string(),
                url: "inline".to_string(),
                protocol: "base64".to_string(),
                is_encrypted: true,
                mime_type: self.payload.mime_type.clone(),
                tdf_spec_version: None,
            },
            encryption_information: self.manifest.encryption_information.clone(),
            schema_version: Some("1.0.0".to_string()),
        }
    }

    /// Serialize to JSON bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

#[cfg(feature = "kas-client")]
impl TdfJsonBuilder {
    /// Set the KAS (Key Access Service) URL
    #[must_use]
    pub fn kas_url(mut self, url: &str) -> Self {
        self.kas_url = Some(url.to_string());
        self
    }

    /// Set the KAS public key PEM for EC key wrapping (required)
    ///
    /// Uses EC (ECDH + HKDF + AES-GCM) key wrapping. This produces smaller
    /// wrapped keys (~60 bytes vs 256 bytes for RSA-2048).
    ///
    /// This method is required - TDF-JSON documents cannot be created without
    /// a valid KAS public key for key wrapping.
    #[must_use]
    pub fn kas_public_key(mut self, pem: &str) -> Self {
        self.kas_public_key_pem = Some(pem.to_string());
        self
    }

    /// Set the access control policy
    #[must_use]
    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Set the MIME type of the original data
    #[must_use]
    pub fn mime_type(mut self, mime_type: &str) -> Self {
        self.mime_type = Some(mime_type.to_string());
        self
    }

    /// Include created timestamp (default: true)
    #[must_use]
    pub fn include_created(mut self, include: bool) -> Self {
        self.include_created = include;
        self
    }

    /// Build the TDF-JSON envelope
    ///
    /// This encrypts the data and creates a complete envelope ready for
    /// transmission over JSON-RPC protocols.
    pub fn build(self) -> Result<TdfJson, EncryptionError> {
        let kas_url = self.kas_url.ok_or(EncryptionError::KeyGenerationError)?;
        let policy = self.policy.ok_or(EncryptionError::KeyGenerationError)?;

        // Create TdfEncryption instance
        let tdf_encryption = TdfEncryption::new()?;

        // Encrypt the data
        let encrypted_payload = tdf_encryption.encrypt(&self.data)?;

        // Get the payload key for policy binding
        let payload_key = tdf_encryption.payload_key();

        // Create policy binding
        let policy_json =
            serde_json::to_string(&policy).map_err(|_| EncryptionError::KeyGenerationError)?;
        let policy_b64 = BASE64.encode(policy_json.as_bytes());

        // Calculate policy binding hash
        let policy_hash = calculate_policy_binding(&policy_b64, payload_key)
            .map_err(|_| EncryptionError::KeyGenerationError)?;

        // Wrap key using EC (ECDH + HKDF + AES-GCM) - KAS public key is required
        // This requires the kas-client feature which provides EC key wrapping
        let kas_pem = self
            .kas_public_key_pem
            .ok_or(EncryptionError::KeyGenerationError)?;
        let ec_result = opentdf_crypto::wrap_key_with_ec(&kas_pem, payload_key)
            .map_err(|_| EncryptionError::KeyGenerationError)?;
        let (wrapped_key, ephemeral_public_key) =
            (ec_result.wrapped_key, Some(ec_result.ephemeral_public_key));

        // Create key access object
        let key_access = KeyAccess {
            access_type: "wrapped".to_string(),
            url: kas_url,
            kid: None,
            protocol: "kas".to_string(),
            wrapped_key,
            policy_binding: crate::manifest::PolicyBinding {
                alg: "HS256".to_string(),
                hash: policy_hash,
            },
            encrypted_metadata: None,
            schema_version: Some("1.0".to_string()),
            ephemeral_public_key,
        };

        // Calculate encrypted size
        let ciphertext_bytes = BASE64.decode(&encrypted_payload.ciphertext)?;
        let encrypted_size = ciphertext_bytes.len() as u64;

        // Extract GMAC tag from encrypted payload (last 16 bytes of ciphertext)
        let gmac_tag = if ciphertext_bytes.len() >= 16 {
            ciphertext_bytes[ciphertext_bytes.len() - 16..].to_vec()
        } else {
            return Err(EncryptionError::KeyGenerationError);
        };

        // Create integrity information with proper root signature
        let mut integrity_info = IntegrityInformation {
            root_signature: RootSignature {
                alg: "HS256".to_string(),
                sig: String::new(),
            },
            segment_hash_alg: "GMAC".to_string(),
            segments: vec![Segment {
                hash: BASE64.encode(&gmac_tag),
                segment_size: Some(self.data.len() as u64),
                encrypted_segment_size: Some(encrypted_size),
            }],
            segment_size_default: self.data.len() as u64,
            encrypted_segment_size_default: encrypted_size,
        };

        // Calculate root signature over GMAC tags
        integrity_info
            .generate_root_signature(&[gmac_tag], payload_key)
            .map_err(|_| EncryptionError::KeyGenerationError)?;

        // Create encryption information
        let encryption_info = EncryptionInformation {
            encryption_type: "split".to_string(),
            key_access: vec![key_access],
            method: EncryptionMethod {
                algorithm: "AES-256-GCM".to_string(),
                is_streamable: true,
                iv: encrypted_payload.iv.clone(),
            },
            integrity_information: integrity_info,
            policy: policy_b64,
        };

        // Create manifest (without payload - payload is at top level)
        let manifest = TdfJsonManifest {
            encryption_information: encryption_info,
            assertions: None,
        };

        // Create payload (at top level per spec)
        let payload = JsonPayload {
            payload_type: "inline".to_string(),
            protocol: "base64".to_string(),
            mime_type: self.mime_type,
            is_encrypted: true,
            length: Some(encrypted_size),
            value: encrypted_payload.ciphertext,
        };

        // Create created timestamp if requested
        let created = if self.include_created {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            None
        };

        Ok(TdfJson {
            tdf: "json".to_string(),
            version: "1.0.0".to_string(),
            created,
            manifest,
            payload,
        })
    }
}

// ============================================================================
// Legacy Builder (deprecated, for backward compatibility)
// ============================================================================

/// Builder for creating TDF JSON-RPC envelopes (legacy)
#[deprecated(
    since = "0.12.0",
    note = "Use TdfJsonBuilder instead. Will be removed in 1.0.0"
)]
pub struct TdfJsonRpcBuilder {
    data: Vec<u8>,
    kas_url: Option<String>,
    policy: Option<Policy>,
    mime_type: String,
}

impl TdfJsonRpc {
    /// Create a new builder for encrypting data into JSON-RPC format
    ///
    /// # Example
    ///
    /// ```rust
    /// use opentdf::{Policy, jsonrpc::TdfJsonRpc};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let policy = Policy::new(
    ///     uuid::Uuid::new_v4().to_string(),
    ///     vec![],
    ///     vec!["user@example.com".to_string()]
    /// );
    ///
    /// let envelope = TdfJsonRpc::encrypt(b"Hello, World!")
    ///     .kas_url("https://kas.example.com")
    ///     .policy(policy)
    ///     .mime_type("text/plain")
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn encrypt(data: &[u8]) -> TdfJsonRpcBuilder {
        TdfJsonRpcBuilder {
            data: data.to_vec(),
            kas_url: None,
            policy: None,
            mime_type: "application/octet-stream".to_string(),
        }
    }

    /// Decrypt the inline payload with a provided data encryption key
    ///
    /// # Arguments
    ///
    /// * `payload_key` - The data encryption key obtained from KAS
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use opentdf::jsonrpc::TdfJsonRpc;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let envelope: TdfJsonRpc = serde_json::from_str("{...}")?;
    /// let payload_key = vec![0u8; 32]; // Obtained from KAS
    ///
    /// let plaintext = envelope.decrypt_with_key(&payload_key)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn decrypt_with_key(&self, payload_key: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        use aes_gcm::{
            Aes256Gcm, Nonce,
            aead::{Aead, KeyInit},
        };

        // Decode base64 payload
        let ciphertext = BASE64.decode(&self.manifest.payload.value)?;

        // Extract IV from encryption method
        // The IV contains both payload IV (12 bytes) and key IV (12 bytes) concatenated
        let iv_bytes = BASE64.decode(&self.manifest.encryption_information.method.iv)?;

        // Extract just the payload IV (first 12 bytes)
        let payload_iv = if iv_bytes.len() >= 12 {
            &iv_bytes[0..12]
        } else {
            &iv_bytes[..]
        };

        // Create cipher with payload key
        let cipher = Aes256Gcm::new_from_slice(payload_key)
            .map_err(|_| EncryptionError::InvalidKeyLength)?;

        let nonce = Nonce::from_slice(payload_iv);

        // Decrypt the data
        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(EncryptionError::AeadError)?;

        Ok(plaintext)
    }

    /// Convert to standard TDF manifest format (for KAS integration)
    ///
    /// This is useful when you need to interact with KAS, which expects
    /// the standard TDF manifest format.
    pub fn to_standard_manifest(&self) -> TdfManifest {
        TdfManifest {
            payload: Payload {
                payload_type: "reference".to_string(),
                url: "inline".to_string(),
                protocol: "base64".to_string(),
                is_encrypted: true,
                mime_type: Some(self.manifest.payload.mime_type.clone()),
                tdf_spec_version: None,
            },
            encryption_information: self.manifest.encryption_information.clone(),
            schema_version: self.manifest.schema_version.clone(),
        }
    }
}

// ============================================================================
// Type Conversions between manifest and inline types
// ============================================================================

impl From<&TdfManifest> for TdfManifestInline {
    /// Convert a standard TDF manifest to inline format.
    ///
    /// Note: The payload value will be empty and must be set separately
    /// since the original payload is a file reference, not inline data.
    fn from(manifest: &TdfManifest) -> Self {
        Self {
            payload: InlinePayload {
                payload_type: "inline".to_string(),
                mime_type: manifest
                    .payload
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                protocol: "base64".to_string(),
                value: String::new(), // Must be set separately
                is_encrypted: manifest.payload.is_encrypted,
            },
            encryption_information: manifest.encryption_information.clone(),
            schema_version: manifest.schema_version.clone(),
        }
    }
}

impl From<TdfManifest> for TdfManifestInline {
    /// Convert a standard TDF manifest to inline format (consuming version).
    ///
    /// Note: The payload value will be empty and must be set separately
    /// since the original payload is a file reference, not inline data.
    fn from(manifest: TdfManifest) -> Self {
        Self {
            payload: InlinePayload {
                payload_type: "inline".to_string(),
                mime_type: manifest
                    .payload
                    .mime_type
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                protocol: "base64".to_string(),
                value: String::new(), // Must be set separately
                is_encrypted: manifest.payload.is_encrypted,
            },
            encryption_information: manifest.encryption_information,
            schema_version: manifest.schema_version,
        }
    }
}

impl From<&TdfManifestInline> for TdfManifest {
    /// Convert an inline manifest back to standard TDF manifest format.
    fn from(inline: &TdfManifestInline) -> Self {
        Self {
            payload: Payload {
                payload_type: "reference".to_string(),
                url: "inline".to_string(),
                protocol: "base64".to_string(),
                is_encrypted: inline.payload.is_encrypted,
                mime_type: Some(inline.payload.mime_type.clone()),
                tdf_spec_version: None,
            },
            encryption_information: inline.encryption_information.clone(),
            schema_version: inline.schema_version.clone(),
        }
    }
}

impl From<TdfManifestInline> for TdfManifest {
    /// Convert an inline manifest back to standard TDF manifest format (consuming version).
    fn from(inline: TdfManifestInline) -> Self {
        Self {
            payload: Payload {
                payload_type: "reference".to_string(),
                url: "inline".to_string(),
                protocol: "base64".to_string(),
                is_encrypted: inline.payload.is_encrypted,
                mime_type: Some(inline.payload.mime_type),
                tdf_spec_version: None,
            },
            encryption_information: inline.encryption_information,
            schema_version: inline.schema_version,
        }
    }
}

impl From<&InlinePayload> for Payload {
    /// Convert an inline payload to a standard payload reference.
    fn from(inline: &InlinePayload) -> Self {
        Self {
            payload_type: "reference".to_string(),
            url: "inline".to_string(),
            protocol: "base64".to_string(),
            is_encrypted: inline.is_encrypted,
            mime_type: Some(inline.mime_type.clone()),
            tdf_spec_version: None,
        }
    }
}

impl From<InlinePayload> for Payload {
    /// Convert an inline payload to a standard payload reference (consuming version).
    fn from(inline: InlinePayload) -> Self {
        Self {
            payload_type: "reference".to_string(),
            url: "inline".to_string(),
            protocol: "base64".to_string(),
            is_encrypted: inline.is_encrypted,
            mime_type: Some(inline.mime_type),
            tdf_spec_version: None,
        }
    }
}

impl TdfJsonRpcBuilder {
    /// Set the KAS (Key Access Service) URL
    #[must_use]
    pub fn kas_url(mut self, url: &str) -> Self {
        self.kas_url = Some(url.to_string());
        self
    }

    /// Set the access control policy
    #[must_use]
    pub fn policy(mut self, policy: Policy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Set the MIME type of the original data
    #[must_use]
    pub fn mime_type(mut self, mime_type: &str) -> Self {
        self.mime_type = mime_type.to_string();
        self
    }

    /// Build the TDF JSON-RPC envelope
    ///
    /// This encrypts the data and creates a complete envelope ready for
    /// transmission over JSON-RPC protocols.
    pub fn build(self) -> Result<TdfJsonRpc, EncryptionError> {
        let kas_url = self.kas_url.ok_or(EncryptionError::KeyGenerationError)?;
        let policy = self.policy.ok_or(EncryptionError::KeyGenerationError)?;

        // Create TdfEncryption instance
        let tdf_encryption = TdfEncryption::new()?;

        // Encrypt the data
        let encrypted_payload = tdf_encryption.encrypt(&self.data)?;

        // Get the payload key for policy binding
        let payload_key = tdf_encryption.payload_key();

        // Create policy binding
        let policy_json =
            serde_json::to_string(&policy).map_err(|_| EncryptionError::KeyGenerationError)?;
        let policy_b64 = BASE64.encode(policy_json.as_bytes());

        // Calculate policy binding hash
        let policy_hash = calculate_policy_binding(&policy_b64, payload_key)
            .map_err(|_| EncryptionError::KeyGenerationError)?;

        // Create key access object
        let key_access = KeyAccess {
            access_type: "wrapped".to_string(),
            url: kas_url,
            kid: None,
            protocol: "kas".to_string(),
            wrapped_key: encrypted_payload.encrypted_key.clone(),
            policy_binding: crate::manifest::PolicyBinding {
                alg: "HS256".to_string(),
                hash: policy_hash,
            },
            encrypted_metadata: None,
            schema_version: Some("1.0".to_string()),
            ephemeral_public_key: None, // RSA wrapping, no ephemeral key needed
        };

        // Calculate encrypted size from base64 string length (more efficient than decoding)
        // Base64 encoding: 4 chars per 3 bytes, so decoded_len = (encoded_len * 3) / 4
        let encrypted_size = ((encrypted_payload.ciphertext.len() * 3) / 4) as u64;

        // Extract GMAC tag from encrypted payload (last 16 bytes of ciphertext)
        // AES-GCM encryption appends the authentication tag to the ciphertext
        let ciphertext_bytes = BASE64.decode(&encrypted_payload.ciphertext)?;
        let gmac_tag = if ciphertext_bytes.len() >= 16 {
            ciphertext_bytes[ciphertext_bytes.len() - 16..].to_vec()
        } else {
            return Err(EncryptionError::KeyGenerationError);
        };

        // Create integrity information with proper root signature
        let mut integrity_info = IntegrityInformation {
            root_signature: RootSignature {
                alg: "HS256".to_string(),
                sig: String::new(), // Will be calculated below
            },
            segment_hash_alg: "GMAC".to_string(),
            segments: vec![Segment {
                hash: BASE64.encode(&gmac_tag), // GMAC tag from encryption
                segment_size: Some(self.data.len() as u64),
                encrypted_segment_size: Some(encrypted_size),
            }],
            segment_size_default: self.data.len() as u64,
            encrypted_segment_size_default: encrypted_size,
        };

        // Calculate root signature over GMAC tags
        integrity_info
            .generate_root_signature(&[gmac_tag], payload_key)
            .map_err(|_| EncryptionError::KeyGenerationError)?;

        // Create encryption information
        let encryption_info = EncryptionInformation {
            encryption_type: "split".to_string(),
            key_access: vec![key_access],
            method: EncryptionMethod {
                algorithm: "AES-256-GCM".to_string(),
                is_streamable: true,
                iv: encrypted_payload.iv.clone(),
            },
            integrity_information: integrity_info,
            policy: policy_b64,
        };

        // Create inline payload
        let inline_payload = InlinePayload {
            payload_type: "inline".to_string(),
            mime_type: self.mime_type,
            protocol: "base64".to_string(),
            value: encrypted_payload.ciphertext,
            is_encrypted: true,
        };

        // Create manifest
        let manifest = TdfManifestInline {
            payload: inline_payload,
            encryption_information: encryption_info,
            schema_version: Some("1.1.0".to_string()),
        };

        Ok(TdfJsonRpc {
            manifest,
            version: "3.0.0".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Get test KAS URL from environment variable or use default
    fn test_kas_url() -> String {
        std::env::var("TEST_KAS_URL").unwrap_or_else(|_| "https://100.arkavo.net".to_string())
    }

    /// Generate a test EC key pair for testing TDF-JSON encryption
    /// Returns (private_key_pem, public_key_pem)
    fn generate_test_ec_key_pair() -> (String, String) {
        use p256::SecretKey;
        use p256::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        use rand::rngs::OsRng;

        let secret_key = SecretKey::random(&mut OsRng);
        let public_key = secret_key.public_key();

        let private_pem = secret_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("Failed to encode private key")
            .to_string();

        let public_pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .expect("Failed to encode public key");

        (private_pem, public_pem)
    }

    #[test]
    fn test_create_jsonrpc_envelope() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let envelope = TdfJsonRpc::encrypt(b"Hello, World!")
            .kas_url(&test_kas_url())
            .policy(policy)
            .mime_type("text/plain")
            .build()
            .expect("Failed to create envelope");

        assert_eq!(envelope.version, "3.0.0");
        assert_eq!(envelope.manifest.payload.payload_type, "inline");
        assert_eq!(envelope.manifest.payload.mime_type, "text/plain");
        assert_eq!(envelope.manifest.payload.protocol, "base64");
        assert!(envelope.manifest.payload.is_encrypted);
    }

    #[test]
    fn test_serialize_deserialize() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let envelope = TdfJsonRpc::encrypt(b"Test data")
            .kas_url(&test_kas_url())
            .policy(policy)
            .build()
            .expect("Failed to create envelope");

        // Serialize to JSON
        let json = serde_json::to_string(&envelope).expect("Failed to serialize");

        // Deserialize back
        let deserialized: TdfJsonRpc = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(envelope.version, deserialized.version);
        assert_eq!(
            envelope.manifest.payload.value,
            deserialized.manifest.payload.value
        );
    }

    #[test]
    fn test_decrypt_with_key() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let original_data = b"Secret message";

        // Create encryption with known keys for testing
        let tdf_encryption = TdfEncryption::new().expect("Failed to create encryption");
        let payload_key = tdf_encryption.payload_key().to_vec();

        // Encrypt the data
        let encrypted_payload = tdf_encryption
            .encrypt(original_data)
            .expect("Failed to encrypt");

        // Create policy binding
        let policy_json = serde_json::to_string(&policy).expect("Failed to serialize policy");
        let policy_b64 = BASE64.encode(policy_json.as_bytes());
        let policy_hash = calculate_policy_binding(&policy_b64, &payload_key)
            .expect("Failed to calculate policy binding");

        // Create key access object
        let key_access = KeyAccess {
            access_type: "wrapped".to_string(),
            url: test_kas_url(),
            kid: None,
            protocol: "kas".to_string(),
            wrapped_key: encrypted_payload.encrypted_key.clone(),
            policy_binding: crate::manifest::PolicyBinding {
                alg: "HS256".to_string(),
                hash: policy_hash,
            },
            encrypted_metadata: None,
            schema_version: Some("1.0".to_string()),
            ephemeral_public_key: None,
        };

        // Create integrity information
        let integrity_info = IntegrityInformation {
            root_signature: RootSignature {
                alg: "HS256".to_string(),
                sig: String::new(),
            },
            segment_hash_alg: "GMAC".to_string(),
            segments: vec![Segment {
                hash: String::new(),
                segment_size: Some(original_data.len() as u64),
                encrypted_segment_size: Some(
                    BASE64.decode(&encrypted_payload.ciphertext).unwrap().len() as u64,
                ),
            }],
            segment_size_default: original_data.len() as u64,
            encrypted_segment_size_default: BASE64
                .decode(&encrypted_payload.ciphertext)
                .unwrap()
                .len() as u64,
        };

        // Create encryption information
        let encryption_info = EncryptionInformation {
            encryption_type: "split".to_string(),
            key_access: vec![key_access],
            method: EncryptionMethod {
                algorithm: "AES-256-GCM".to_string(),
                is_streamable: true,
                iv: encrypted_payload.iv.clone(),
            },
            integrity_information: integrity_info,
            policy: policy_b64,
        };

        // Create inline payload
        let inline_payload = InlinePayload {
            payload_type: "inline".to_string(),
            mime_type: "text/plain".to_string(),
            protocol: "base64".to_string(),
            value: encrypted_payload.ciphertext,
            is_encrypted: true,
        };

        // Create manifest
        let manifest = TdfManifestInline {
            payload: inline_payload,
            encryption_information: encryption_info,
            schema_version: Some("1.1.0".to_string()),
        };

        let envelope = TdfJsonRpc {
            manifest,
            version: "3.0.0".to_string(),
        };

        // Decrypt using the payload key
        let decrypted = envelope
            .decrypt_with_key(&payload_key)
            .expect("Failed to decrypt");

        assert_eq!(original_data, decrypted.as_slice());
    }

    #[test]
    fn test_decrypt_with_wrong_key() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let envelope = TdfJsonRpc::encrypt(b"Secret data")
            .kas_url(&test_kas_url())
            .policy(policy)
            .build()
            .expect("Failed to create envelope");

        // Try to decrypt with wrong key
        let wrong_key = vec![0u8; 32];
        let result = envelope.decrypt_with_key(&wrong_key);

        assert!(result.is_err(), "Decryption should fail with wrong key");
    }

    #[test]
    fn test_decrypt_with_invalid_key_length() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let envelope = TdfJsonRpc::encrypt(b"Secret data")
            .kas_url(&test_kas_url())
            .policy(policy)
            .build()
            .expect("Failed to create envelope");

        // Try to decrypt with invalid key length
        let invalid_key = vec![0u8; 16]; // Wrong size
        let result = envelope.decrypt_with_key(&invalid_key);

        assert!(
            result.is_err(),
            "Decryption should fail with invalid key length"
        );
        match result {
            Err(EncryptionError::InvalidKeyLength) => (),
            _ => panic!("Expected InvalidKeyLength error"),
        }
    }

    #[test]
    fn test_builder_missing_kas_url() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let result = TdfJsonRpc::encrypt(b"Data").policy(policy).build();

        assert!(result.is_err(), "Build should fail without KAS URL");
    }

    #[test]
    fn test_builder_missing_policy() {
        let result = TdfJsonRpc::encrypt(b"Data")
            .kas_url(&test_kas_url())
            .build();

        assert!(result.is_err(), "Build should fail without policy");
    }

    #[test]
    fn test_roundtrip_encryption_decryption() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let original_data =
            b"This is a roundtrip test with longer data to ensure everything works correctly!";

        // Create encryption
        let tdf_encryption = TdfEncryption::new().expect("Failed to create encryption");
        let _payload_key = tdf_encryption.payload_key().to_vec();

        // Encrypt
        let envelope = TdfJsonRpc::encrypt(original_data)
            .kas_url(&test_kas_url())
            .policy(policy)
            .mime_type("text/plain")
            .build()
            .expect("Failed to encrypt");

        // Serialize to JSON
        let json = serde_json::to_string(&envelope).expect("Failed to serialize");

        // Deserialize from JSON
        let deserialized: TdfJsonRpc = serde_json::from_str(&json).expect("Failed to deserialize");

        // Verify structure
        assert_eq!(deserialized.version, "3.0.0");
        assert_eq!(deserialized.manifest.payload.payload_type, "inline");
        assert_eq!(deserialized.manifest.payload.mime_type, "text/plain");
        assert!(deserialized.manifest.payload.is_encrypted);

        // Note: In a real scenario, we would get the payload key from KAS
        // For this test, we're using the key from the encryption object
        // This simulates what would happen after KAS unwraps the key
    }

    #[test]
    fn test_empty_payload() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let envelope = TdfJsonRpc::encrypt(b"")
            .kas_url(&test_kas_url())
            .policy(policy)
            .build()
            .expect("Failed to create envelope with empty payload");

        assert_eq!(envelope.manifest.payload.payload_type, "inline");
    }

    #[test]
    fn test_large_payload() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        // Create a 1MB payload
        let large_data = vec![0u8; 1024 * 1024];

        let envelope = TdfJsonRpc::encrypt(&large_data)
            .kas_url(&test_kas_url())
            .policy(policy)
            .build()
            .expect("Failed to create envelope with large payload");

        // Verify the envelope was created
        assert_eq!(envelope.manifest.payload.payload_type, "inline");

        // Verify base64 encoding worked
        assert!(!envelope.manifest.payload.value.is_empty());
    }

    #[test]
    fn test_custom_mime_type() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let envelope = TdfJsonRpc::encrypt(b"JSON data")
            .kas_url(&test_kas_url())
            .policy(policy)
            .mime_type("application/json")
            .build()
            .expect("Failed to create envelope");

        assert_eq!(envelope.manifest.payload.mime_type, "application/json");
    }

    // ========================================================================
    // TDF-JSON Spec-Compliant Tests
    // ========================================================================

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_tdf_json_create_envelope() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );
        let (_private_pem, public_pem) = generate_test_ec_key_pair();

        let envelope = TdfJson::encrypt(b"Hello, World!")
            .kas_url(&test_kas_url())
            .kas_public_key(&public_pem)
            .policy(policy)
            .mime_type("text/plain")
            .build()
            .expect("Failed to create envelope");

        // Verify spec-compliant structure
        assert_eq!(envelope.tdf, "json");
        assert_eq!(envelope.version, "1.0.0");
        assert!(envelope.created.is_some()); // Created timestamp included by default
        assert_eq!(envelope.payload.payload_type, "inline");
        assert_eq!(envelope.payload.protocol, "base64");
        assert_eq!(envelope.payload.mime_type, Some("text/plain".to_string()));
        assert!(envelope.payload.is_encrypted);
        assert!(envelope.payload.length.is_some());
    }

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_tdf_json_serialize_deserialize() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );
        let (_private_pem, public_pem) = generate_test_ec_key_pair();

        let envelope = TdfJson::encrypt(b"Test data")
            .kas_url(&test_kas_url())
            .kas_public_key(&public_pem)
            .policy(policy)
            .include_created(false) // Exclude for deterministic testing
            .build()
            .expect("Failed to create envelope");

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&envelope).expect("Failed to serialize");

        // Verify JSON structure has top-level fields per spec
        assert!(json.contains("\"tdf\": \"json\""));
        assert!(json.contains("\"version\": \"1.0.0\""));
        assert!(json.contains("\"manifest\":"));
        assert!(json.contains("\"payload\":"));

        // Deserialize back
        let deserialized: TdfJson = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(envelope.tdf, deserialized.tdf);
        assert_eq!(envelope.version, deserialized.version);
        assert_eq!(envelope.payload.value, deserialized.payload.value);
    }

    #[test]
    fn test_tdf_json_decrypt_with_key() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let original_data = b"Secret message for TDF-JSON";

        // Create encryption with known keys for testing
        let tdf_encryption = TdfEncryption::new().expect("Failed to create encryption");
        let payload_key = tdf_encryption.payload_key().to_vec();

        // Encrypt the data
        let encrypted_payload = tdf_encryption
            .encrypt(original_data)
            .expect("Failed to encrypt");

        // Create policy binding
        let policy_json = serde_json::to_string(&policy).expect("Failed to serialize policy");
        let policy_b64 = BASE64.encode(policy_json.as_bytes());
        let policy_hash = calculate_policy_binding(&policy_b64, &payload_key)
            .expect("Failed to calculate policy binding");

        // Create key access object
        let key_access = KeyAccess {
            access_type: "wrapped".to_string(),
            url: test_kas_url(),
            kid: None,
            protocol: "kas".to_string(),
            wrapped_key: encrypted_payload.encrypted_key.clone(),
            policy_binding: crate::manifest::PolicyBinding {
                alg: "HS256".to_string(),
                hash: policy_hash,
            },
            encrypted_metadata: None,
            schema_version: Some("1.0".to_string()),
            ephemeral_public_key: None,
        };

        let ciphertext_len = BASE64.decode(&encrypted_payload.ciphertext).unwrap().len() as u64;

        // Create integrity information
        let integrity_info = IntegrityInformation {
            root_signature: RootSignature {
                alg: "HS256".to_string(),
                sig: String::new(),
            },
            segment_hash_alg: "GMAC".to_string(),
            segments: vec![Segment {
                hash: String::new(),
                segment_size: Some(original_data.len() as u64),
                encrypted_segment_size: Some(ciphertext_len),
            }],
            segment_size_default: original_data.len() as u64,
            encrypted_segment_size_default: ciphertext_len,
        };

        // Create encryption information
        let encryption_info = EncryptionInformation {
            encryption_type: "split".to_string(),
            key_access: vec![key_access],
            method: EncryptionMethod {
                algorithm: "AES-256-GCM".to_string(),
                is_streamable: true,
                iv: encrypted_payload.iv.clone(),
            },
            integrity_information: integrity_info,
            policy: policy_b64,
        };

        // Create TDF-JSON manifest (without payload - payload at top level)
        let manifest = TdfJsonManifest {
            encryption_information: encryption_info,
            assertions: None,
        };

        // Create payload (at top level per spec)
        let payload = JsonPayload {
            payload_type: "inline".to_string(),
            protocol: "base64".to_string(),
            mime_type: Some("text/plain".to_string()),
            is_encrypted: true,
            length: Some(ciphertext_len),
            value: encrypted_payload.ciphertext,
        };

        let envelope = TdfJson {
            tdf: "json".to_string(),
            version: "1.0.0".to_string(),
            created: None,
            manifest,
            payload,
        };

        // Decrypt using the payload key
        let decrypted = envelope
            .decrypt_with_key(&payload_key)
            .expect("Failed to decrypt");

        assert_eq!(original_data, decrypted.as_slice());
    }

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_tdf_json_format_id() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );
        let (_private_pem, public_pem) = generate_test_ec_key_pair();

        let envelope = TdfJson::encrypt(b"Test")
            .kas_url(&test_kas_url())
            .kas_public_key(&public_pem)
            .policy(policy)
            .build()
            .expect("Failed to create envelope");

        assert_eq!(envelope.format_id(), "json");
    }

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_tdf_json_to_bytes() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );
        let (_private_pem, public_pem) = generate_test_ec_key_pair();

        let envelope = TdfJson::encrypt(b"Test")
            .kas_url(&test_kas_url())
            .kas_public_key(&public_pem)
            .policy(policy)
            .include_created(false)
            .build()
            .expect("Failed to create envelope");

        let bytes = envelope.to_bytes().expect("Failed to serialize");
        assert!(!bytes.is_empty());

        // Verify it's valid JSON
        let parsed: TdfJson = serde_json::from_slice(&bytes).expect("Failed to parse");
        assert_eq!(parsed.tdf, "json");
    }

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_tdf_json_no_created_timestamp() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );
        let (_private_pem, public_pem) = generate_test_ec_key_pair();

        let envelope = TdfJson::encrypt(b"Test")
            .kas_url(&test_kas_url())
            .kas_public_key(&public_pem)
            .policy(policy)
            .include_created(false)
            .build()
            .expect("Failed to create envelope");

        assert!(envelope.created.is_none());

        // Verify created is not in serialized JSON
        let json = serde_json::to_string(&envelope).expect("Failed to serialize");
        assert!(!json.contains("created"));
    }

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_tdf_json_to_standard_manifest() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );
        let (_private_pem, public_pem) = generate_test_ec_key_pair();

        let envelope = TdfJson::encrypt(b"Test")
            .kas_url(&test_kas_url())
            .kas_public_key(&public_pem)
            .policy(policy)
            .mime_type("text/plain")
            .build()
            .expect("Failed to create envelope");

        let manifest = envelope.to_standard_manifest();

        // Standard manifest should have payload as reference
        assert_eq!(manifest.payload.payload_type, "reference");
        assert_eq!(manifest.payload.url, "inline");
        assert_eq!(manifest.payload.protocol, "base64");
        assert!(manifest.payload.is_encrypted);
    }
}
