//! TDF-CBOR format implementation (per TDF-CBOR specification draft-00)
//!
//! This module provides support for CBOR-encoded TDF payloads optimized for
//! binary protocols and size-constrained environments.
//!
//! # Overview
//!
//! TDF-CBOR defines a CBOR-based container format for TDF that:
//! - Uses integer keys for compact encoding
//! - Stores binary payloads directly (no base64)
//! - Includes self-describe CBOR tag for format detection
//!
//! # Example
//!
//! ```rust,no_run
//! # #[cfg(feature = "cbor")]
//! use opentdf::{Policy, tdf_cbor::TdfCbor};
//!
//! # #[cfg(feature = "cbor")]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a policy
//! let policy = opentdf::Policy::new(
//!     uuid::Uuid::new_v4().to_string(),
//!     vec![],
//!     vec!["user@example.com".to_string()]
//! );
//!
//! // Encrypt data and get TDF-CBOR container
//! let container = TdfCbor::encrypt(b"Sensitive data")
//!     .kas_url("https://kas.example.com")
//!     .policy(policy)
//!     .build()?;
//!
//! // Serialize to CBOR bytes
//! let cbor_bytes = container.to_bytes()?;
//!
//! // Later: parse from bytes
//! let parsed = TdfCbor::from_bytes(&cbor_bytes)?;
//! assert_eq!(parsed.tdf, "cbor");
//! assert_eq!(parsed.version, [1, 0, 0]);
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "cbor"))]
//! # fn main() {}
//! ```

use crate::manifest::{
    EncryptionInformation, EncryptionMethod, IntegrityInformation, IntegrityInformationExt,
    KeyAccess, RootSignature, Segment,
};
use crate::policy::Policy;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use opentdf_crypto::{EncryptionError, TdfEncryption, calculate_policy_binding};
use serde::{Deserialize, Serialize};

// ============================================================================
// TDF-CBOR Magic Bytes
// ============================================================================

/// Self-describe CBOR tag (55799) + map marker
/// D9 D9F7 = tag(55799)
/// A5 = map(5) for 5 top-level elements
pub const CBOR_MAGIC: [u8; 4] = [0xD9, 0xD9, 0xF7, 0xA5];

/// Integer key mappings per TDF-CBOR spec section 3.1
pub mod key {
    pub const TDF: u64 = 1;
    pub const VERSION: u64 = 2;
    pub const CREATED: u64 = 3;
    pub const MANIFEST: u64 = 4;
    pub const PAYLOAD: u64 = 5;
}

/// Payload integer key mappings per TDF-CBOR spec section 3.1
pub mod payload_key {
    pub const TYPE: u64 = 1;
    pub const PROTOCOL: u64 = 2;
    pub const MIME_TYPE: u64 = 3;
    pub const IS_ENCRYPTED: u64 = 4;
    pub const LENGTH: u64 = 5;
    pub const VALUE: u64 = 6;
}

/// Enumerated values per TDF-CBOR spec section 1.5
pub mod enums {
    // Payload type: 0=inline, 1=reference
    pub const PAYLOAD_TYPE_INLINE: u64 = 0;
    #[allow(dead_code)]
    pub const PAYLOAD_TYPE_REFERENCE: u64 = 1;

    // Payload protocol: 0=binary, 1=binary-chunked
    pub const PAYLOAD_PROTOCOL_BINARY: u64 = 0;
    #[allow(dead_code)]
    pub const PAYLOAD_PROTOCOL_BINARY_CHUNKED: u64 = 1;

    // Encryption type: 0=split
    pub const ENCRYPTION_TYPE_SPLIT: u64 = 0;

    // Key access type: 0=wrapped, 1=remote
    pub const KEY_ACCESS_TYPE_WRAPPED: u64 = 0;
    #[allow(dead_code)]
    pub const KEY_ACCESS_TYPE_REMOTE: u64 = 1;

    // Key protocol: 0=kas
    pub const KEY_PROTOCOL_KAS: u64 = 0;

    // Symmetric algorithm: 0=AES-256-GCM
    pub const SYMMETRIC_ALG_AES_256_GCM: u64 = 0;

    // Hash/Signature algorithm
    pub const HASH_ALG_HS256: u64 = 0;
    #[allow(dead_code)]
    pub const HASH_ALG_HS384: u64 = 1;
    #[allow(dead_code)]
    pub const HASH_ALG_HS512: u64 = 2;
    pub const HASH_ALG_GMAC: u64 = 3;
    #[allow(dead_code)]
    pub const HASH_ALG_SHA256: u64 = 4;
    #[allow(dead_code)]
    pub const HASH_ALG_ES256: u64 = 5;
    #[allow(dead_code)]
    pub const HASH_ALG_ES384: u64 = 6;
    #[allow(dead_code)]
    pub const HASH_ALG_ES512: u64 = 7;
}

/// Manifest integer key mappings per TDF-CBOR spec section 3.1
pub mod manifest_key {
    pub const ENCRYPTION_INFORMATION: u64 = 1;
    #[allow(dead_code)]
    pub const ASSERTIONS: u64 = 2;
}

/// EncryptionInformation integer key mappings
pub mod enc_info_key {
    pub const TYPE: u64 = 1;
    pub const KEY_ACCESS: u64 = 2;
    pub const METHOD: u64 = 3;
    pub const INTEGRITY_INFORMATION: u64 = 4;
    pub const POLICY: u64 = 5;
}

/// KeyAccess integer key mappings
pub mod key_access_key {
    pub const TYPE: u64 = 1;
    pub const URL: u64 = 2;
    pub const PROTOCOL: u64 = 3;
    pub const WRAPPED_KEY: u64 = 4;
    pub const POLICY_BINDING: u64 = 5;
    #[allow(dead_code)]
    pub const ENCRYPTED_METADATA: u64 = 6;
    pub const KID: u64 = 7;
    pub const EPHEMERAL_PUBLIC_KEY: u64 = 8;
    pub const SCHEMA_VERSION: u64 = 9;
}

/// PolicyBinding integer key mappings
pub mod policy_binding_key {
    pub const ALG: u64 = 1;
    pub const HASH: u64 = 2;
}

/// Method integer key mappings
pub mod method_key {
    pub const ALGORITHM: u64 = 1;
    pub const IV: u64 = 2;
    pub const IS_STREAMABLE: u64 = 3;
}

/// IntegrityInformation integer key mappings
pub mod integrity_key {
    pub const ROOT_SIGNATURE: u64 = 1;
    pub const SEGMENT_HASH_ALG: u64 = 2;
    pub const SEGMENTS: u64 = 3;
    pub const SEGMENT_SIZE_DEFAULT: u64 = 4;
    pub const ENCRYPTED_SEGMENT_SIZE_DEFAULT: u64 = 5;
}

/// RootSignature integer key mappings
pub mod root_sig_key {
    pub const ALG: u64 = 1;
    pub const SIG: u64 = 2;
}

/// Segment integer key mappings
pub mod segment_key {
    pub const HASH: u64 = 1;
    pub const SEGMENT_SIZE: u64 = 2;
    pub const ENCRYPTED_SEGMENT_SIZE: u64 = 3;
}

// ============================================================================
// TDF-CBOR Spec-Compliant Types
// ============================================================================

/// TDF-CBOR envelope for binary payload transmission (spec-compliant)
///
/// This structure represents a complete TDF-CBOR package per the TDF-CBOR
/// specification draft-00. The format uses integer keys and binary payloads
/// for optimal size and parsing efficiency.
///
/// # Integer Key Mapping
///
/// | Key | Field    | Type                |
/// |-----|----------|---------------------|
/// | 1   | tdf      | string "cbor"       |
/// | 2   | version  | [u8; 3] semver      |
/// | 3   | created  | u64 Unix timestamp  |
/// | 4   | manifest | TdfCborManifest     |
/// | 5   | payload  | CborPayload         |
#[derive(Debug, Clone)]
pub struct TdfCbor {
    /// Format identifier. MUST be "cbor" for TDF-CBOR documents.
    pub tdf: String,

    /// Semantic version as [major, minor, patch] array
    pub version: [u8; 3],

    /// Unix timestamp of document creation (optional)
    pub created: Option<u64>,

    /// TDF manifest containing encryption and policy information
    pub manifest: TdfCborManifest,

    /// Binary encrypted payload container
    pub payload: CborPayload,
}

/// TDF manifest for TDF-CBOR format
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TdfCborManifest {
    /// Encryption information including key access and policy
    pub encryption_information: EncryptionInformation,

    /// Optional assertions for additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertions: Option<Vec<serde_json::Value>>,
}

/// Binary payload for TDF-CBOR transport
///
/// Contains the encrypted data directly as binary bytes (no base64 encoding).
#[derive(Debug, Clone)]
pub struct CborPayload {
    /// Payload type. MUST be "inline" for TDF-CBOR
    pub payload_type: String,

    /// Protocol. MUST be "binary" for TDF-CBOR (not "base64")
    pub protocol: String,

    /// MIME type of the original (unencrypted) data
    pub mime_type: Option<String>,

    /// Whether the payload is encrypted. MUST be true
    pub is_encrypted: bool,

    /// Raw encrypted bytes (not base64 encoded)
    pub value: Vec<u8>,
}

// ============================================================================
// TDF-CBOR Error Types
// ============================================================================

/// Errors specific to TDF-CBOR parsing and validation
#[derive(Debug)]
pub enum TdfCborError {
    /// Invalid or missing CBOR magic bytes
    InvalidMagicBytes,
    /// Invalid tdf identifier
    InvalidTdfIdentifier(String),
    /// Unexpected integer key in CBOR structure
    UnexpectedKey { expected: u64, got: u64 },
    /// CBOR encoding error
    EncodingError(String),
    /// CBOR decoding error
    DecodingError(String),
    /// Missing required field
    MissingField(String),
    /// Encryption error
    EncryptionError(EncryptionError),
}

impl std::fmt::Display for TdfCborError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidMagicBytes => write!(f, "Invalid or missing CBOR magic bytes"),
            Self::InvalidTdfIdentifier(s) => {
                write!(f, "Invalid tdf identifier: expected 'cbor', got '{}'", s)
            }
            Self::UnexpectedKey { expected, got } => {
                write!(f, "Expected integer key {}, got {}", expected, got)
            }
            Self::EncodingError(e) => write!(f, "CBOR encoding error: {}", e),
            Self::DecodingError(e) => write!(f, "CBOR decoding error: {}", e),
            Self::MissingField(s) => write!(f, "Missing required field: {}", s),
            Self::EncryptionError(e) => write!(f, "Encryption error: {}", e),
        }
    }
}

impl std::error::Error for TdfCborError {}

impl From<EncryptionError> for TdfCborError {
    fn from(e: EncryptionError) -> Self {
        Self::EncryptionError(e)
    }
}

impl From<base64::DecodeError> for TdfCborError {
    fn from(e: base64::DecodeError) -> Self {
        Self::DecodingError(format!("Base64 decode error: {}", e))
    }
}

// ============================================================================
// TDF-CBOR Builder
// ============================================================================

/// Builder for creating TDF-CBOR containers
///
/// Requires the `kas-client` feature for EC key wrapping support.
#[cfg(feature = "kas-client")]
pub struct TdfCborBuilder {
    data: Vec<u8>,
    kas_url: Option<String>,
    kas_public_key_pem: Option<String>,
    policy: Option<Policy>,
    mime_type: Option<String>,
    include_created: bool,
}

impl TdfCbor {
    /// Create a new builder for encrypting data into TDF-CBOR format
    ///
    /// Requires the `kas-client` feature for EC key wrapping support.
    #[cfg(feature = "kas-client")]
    pub fn encrypt(data: &[u8]) -> TdfCborBuilder {
        TdfCborBuilder {
            data: data.to_vec(),
            kas_url: None,
            kas_public_key_pem: None,
            policy: None,
            mime_type: None,
            include_created: true,
        }
    }

    /// Format identifier (always "cbor")
    pub fn format_id(&self) -> &str {
        &self.tdf
    }

    /// Check if data starts with CBOR self-describe tag
    ///
    /// The self-describe CBOR tag is 0xD9 0xD9 0xF7 (tag 55799)
    pub fn has_magic_bytes(data: &[u8]) -> bool {
        data.len() >= 3 && data[0] == 0xD9 && data[1] == 0xD9 && data[2] == 0xF7
    }

    /// Parse TDF-CBOR from bytes
    #[cfg(feature = "cbor")]
    pub fn from_bytes(data: &[u8]) -> Result<Self, TdfCborError> {
        use ciborium::Value;

        // Verify magic bytes (at least the self-describe tag)
        if data.len() < 3 || data[0] != 0xD9 || data[1] != 0xD9 || data[2] != 0xF7 {
            return Err(TdfCborError::InvalidMagicBytes);
        }

        // Parse CBOR
        let value: Value = ciborium::from_reader(&data[3..])
            .map_err(|e| TdfCborError::DecodingError(e.to_string()))?;

        // Extract map
        let map = match value {
            Value::Map(m) => m,
            _ => return Err(TdfCborError::DecodingError("Expected CBOR map".to_string())),
        };

        // Parse fields by integer key
        let mut tdf = None;
        let mut version = None;
        let mut created = None;
        let mut manifest = None;
        let mut payload = None;

        for (k, v) in map {
            let key = match k {
                Value::Integer(i) => {
                    let i128_val: i128 = i.into();
                    i128_val as u64
                }
                _ => continue,
            };

            match key {
                key::TDF => {
                    if let Value::Text(s) = v {
                        tdf = Some(s);
                    }
                }
                key::VERSION => {
                    if let Value::Array(arr) = v
                        && arr.len() == 3
                    {
                        let mut ver = [0u8; 3];
                        for (i, val) in arr.iter().enumerate() {
                            if let Value::Integer(n) = val {
                                let i128_val: i128 = (*n).into();
                                ver[i] = i128_val as u8;
                            }
                        }
                        version = Some(ver);
                    }
                }
                key::CREATED => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        created = Some(i128_val as u64);
                    }
                }
                key::MANIFEST => {
                    // Support both native CBOR map (new) and JSON string (legacy)
                    match v {
                        Value::Map(_) => {
                            // Native CBOR manifest with integer keys
                            let m = Self::decode_manifest_from_cbor(v)?;
                            manifest = Some(m);
                        }
                        Value::Text(s) => {
                            // Legacy JSON string format
                            let m: TdfCborManifest = serde_json::from_str(&s)
                                .map_err(|e| TdfCborError::DecodingError(e.to_string()))?;
                            manifest = Some(m);
                        }
                        _ => {}
                    }
                }
                key::PAYLOAD => {
                    if let Value::Map(payload_map) = v {
                        let mut payload_type = String::new();
                        let mut protocol = String::new();
                        let mut mime_type = None;
                        let mut is_encrypted = true;
                        let mut value = Vec::new();

                        for (pk, pv) in payload_map {
                            // Support both integer keys (new spec) and string keys (legacy)
                            let pkey: u64 = match pk {
                                Value::Integer(i) => {
                                    let i128_val: i128 = i.into();
                                    i128_val as u64
                                }
                                Value::Text(s) => match s.as_str() {
                                    "type" => payload_key::TYPE,
                                    "protocol" => payload_key::PROTOCOL,
                                    "mimeType" => payload_key::MIME_TYPE,
                                    "isEncrypted" => payload_key::IS_ENCRYPTED,
                                    "value" => payload_key::VALUE,
                                    _ => continue,
                                },
                                _ => continue,
                            };

                            match pkey {
                                payload_key::TYPE => {
                                    // Support both integer enum (new) and string (legacy)
                                    match pv {
                                        Value::Integer(i) => {
                                            let i128_val: i128 = i.into();
                                            payload_type = match i128_val as u64 {
                                                enums::PAYLOAD_TYPE_INLINE => "inline".to_string(),
                                                enums::PAYLOAD_TYPE_REFERENCE => {
                                                    "reference".to_string()
                                                }
                                                _ => "inline".to_string(),
                                            };
                                        }
                                        Value::Text(s) => payload_type = s,
                                        _ => {}
                                    }
                                }
                                payload_key::PROTOCOL => {
                                    // Support both integer enum (new) and string (legacy)
                                    match pv {
                                        Value::Integer(i) => {
                                            let i128_val: i128 = i.into();
                                            protocol = match i128_val as u64 {
                                                enums::PAYLOAD_PROTOCOL_BINARY => {
                                                    "binary".to_string()
                                                }
                                                enums::PAYLOAD_PROTOCOL_BINARY_CHUNKED => {
                                                    "binary-chunked".to_string()
                                                }
                                                _ => "binary".to_string(),
                                            };
                                        }
                                        Value::Text(s) => protocol = s,
                                        _ => {}
                                    }
                                }
                                payload_key::MIME_TYPE => {
                                    if let Value::Text(s) = pv {
                                        mime_type = Some(s);
                                    }
                                }
                                payload_key::IS_ENCRYPTED => {
                                    if let Value::Bool(b) = pv {
                                        is_encrypted = b;
                                    }
                                }
                                payload_key::VALUE => {
                                    if let Value::Bytes(b) = pv {
                                        value = b;
                                    }
                                }
                                _ => {}
                            }
                        }

                        payload = Some(CborPayload {
                            payload_type,
                            protocol,
                            mime_type,
                            is_encrypted,
                            value,
                        });
                    }
                }
                _ => {}
            }
        }

        let tdf_value = tdf.ok_or_else(|| TdfCborError::MissingField("tdf".to_string()))?;
        if tdf_value != "cbor" {
            return Err(TdfCborError::InvalidTdfIdentifier(tdf_value));
        }

        Ok(TdfCbor {
            tdf: tdf_value,
            version: version.ok_or_else(|| TdfCborError::MissingField("version".to_string()))?,
            created,
            manifest: manifest.ok_or_else(|| TdfCborError::MissingField("manifest".to_string()))?,
            payload: payload.ok_or_else(|| TdfCborError::MissingField("payload".to_string()))?,
        })
    }

    /// Serialize to CBOR bytes with magic header
    #[cfg(feature = "cbor")]
    pub fn to_bytes(&self) -> Result<Vec<u8>, TdfCborError> {
        use ciborium::Value;

        // Build the manifest as native CBOR with integer keys and enums
        let manifest_cbor = self.encode_manifest_to_cbor()?;

        let mut map = vec![
            (
                Value::Integer(key::TDF.into()),
                Value::Text(self.tdf.clone()),
            ),
            (
                Value::Integer(key::VERSION.into()),
                Value::Array(vec![
                    Value::Integer(self.version[0].into()),
                    Value::Integer(self.version[1].into()),
                    Value::Integer(self.version[2].into()),
                ]),
            ),
        ];

        if let Some(ts) = self.created {
            map.push((
                Value::Integer(key::CREATED.into()),
                Value::Integer(ts.into()),
            ));
        }

        map.push((Value::Integer(key::MANIFEST.into()), manifest_cbor));

        // Build payload map with integer keys and enum values per spec section 1.5
        let payload_type_enum = match self.payload.payload_type.as_str() {
            "inline" => enums::PAYLOAD_TYPE_INLINE,
            "reference" => enums::PAYLOAD_TYPE_REFERENCE,
            _ => enums::PAYLOAD_TYPE_INLINE,
        };
        let protocol_enum = match self.payload.protocol.as_str() {
            "binary" => enums::PAYLOAD_PROTOCOL_BINARY,
            "binary-chunked" => enums::PAYLOAD_PROTOCOL_BINARY_CHUNKED,
            _ => enums::PAYLOAD_PROTOCOL_BINARY,
        };

        let mut payload_map = vec![
            (
                Value::Integer(payload_key::TYPE.into()),
                Value::Integer(payload_type_enum.into()),
            ),
            (
                Value::Integer(payload_key::PROTOCOL.into()),
                Value::Integer(protocol_enum.into()),
            ),
            (
                Value::Integer(payload_key::IS_ENCRYPTED.into()),
                Value::Bool(self.payload.is_encrypted),
            ),
            (
                Value::Integer(payload_key::VALUE.into()),
                Value::Bytes(self.payload.value.clone()),
            ),
        ];

        if let Some(ref mt) = self.payload.mime_type {
            payload_map.push((
                Value::Integer(payload_key::MIME_TYPE.into()),
                Value::Text(mt.clone()),
            ));
        }

        map.push((Value::Integer(key::PAYLOAD.into()), Value::Map(payload_map)));

        let cbor_value = Value::Map(map);

        // Encode with self-describe tag
        let mut result = vec![0xD9, 0xD9, 0xF7]; // Self-describe CBOR tag(55799)
        ciborium::into_writer(&cbor_value, &mut result)
            .map_err(|e| TdfCborError::EncodingError(e.to_string()))?;

        Ok(result)
    }

    /// Encode manifest to native CBOR with integer keys and enums
    #[cfg(feature = "cbor")]
    fn encode_manifest_to_cbor(&self) -> Result<ciborium::Value, TdfCborError> {
        use ciborium::Value;

        let enc_info = &self.manifest.encryption_information;

        // Encode policy as raw bytes (decode from base64)
        let policy_bytes = BASE64
            .decode(&enc_info.policy)
            .map_err(|e| TdfCborError::EncodingError(format!("Invalid policy base64: {}", e)))?;

        // Encode key access array
        let key_access_array: Vec<Value> = enc_info
            .key_access
            .iter()
            .map(|ka| self.encode_key_access_to_cbor(ka))
            .collect::<Result<Vec<_>, _>>()?;

        // Encode method
        let method_cbor = self.encode_method_to_cbor(&enc_info.method)?;

        // Encode integrity information
        let integrity_cbor = self.encode_integrity_to_cbor(&enc_info.integrity_information)?;

        // Encryption type enum: "split" -> 0
        let enc_type_enum = match enc_info.encryption_type.as_str() {
            "split" => enums::ENCRYPTION_TYPE_SPLIT,
            _ => enums::ENCRYPTION_TYPE_SPLIT,
        };

        // Build encryptionInformation map
        let enc_info_map = vec![
            (
                Value::Integer(enc_info_key::TYPE.into()),
                Value::Integer(enc_type_enum.into()),
            ),
            (
                Value::Integer(enc_info_key::KEY_ACCESS.into()),
                Value::Array(key_access_array),
            ),
            (Value::Integer(enc_info_key::METHOD.into()), method_cbor),
            (
                Value::Integer(enc_info_key::INTEGRITY_INFORMATION.into()),
                integrity_cbor,
            ),
            (
                Value::Integer(enc_info_key::POLICY.into()),
                Value::Bytes(policy_bytes),
            ),
        ];

        // Build manifest map
        let manifest_map = vec![(
            Value::Integer(manifest_key::ENCRYPTION_INFORMATION.into()),
            Value::Map(enc_info_map),
        )];

        Ok(Value::Map(manifest_map))
    }

    /// Encode a single key access object to CBOR
    #[cfg(feature = "cbor")]
    fn encode_key_access_to_cbor(&self, ka: &KeyAccess) -> Result<ciborium::Value, TdfCborError> {
        use ciborium::Value;

        // Key access type enum: "wrapped" -> 0, "remote" -> 1
        let ka_type_enum = match ka.access_type.as_str() {
            "wrapped" => enums::KEY_ACCESS_TYPE_WRAPPED,
            "remote" => enums::KEY_ACCESS_TYPE_REMOTE,
            _ => enums::KEY_ACCESS_TYPE_WRAPPED,
        };

        // Protocol enum: "kas" -> 0
        let protocol_enum = match ka.protocol.as_str() {
            "kas" => enums::KEY_PROTOCOL_KAS,
            _ => enums::KEY_PROTOCOL_KAS,
        };

        // Decode wrapped key from base64 to raw bytes
        let wrapped_key_bytes = BASE64.decode(&ka.wrapped_key).map_err(|e| {
            TdfCborError::EncodingError(format!("Invalid wrappedKey base64: {}", e))
        })?;

        // Encode policy binding
        let binding = &ka.policy_binding;
        let binding_alg_enum = self.hash_alg_to_enum(&binding.alg);
        let binding_hash_bytes = BASE64.decode(&binding.hash).map_err(|e| {
            TdfCborError::EncodingError(format!("Invalid binding hash base64: {}", e))
        })?;

        let policy_binding_map = vec![
            (
                Value::Integer(policy_binding_key::ALG.into()),
                Value::Integer(binding_alg_enum.into()),
            ),
            (
                Value::Integer(policy_binding_key::HASH.into()),
                Value::Bytes(binding_hash_bytes),
            ),
        ];

        let mut ka_map = vec![
            (
                Value::Integer(key_access_key::TYPE.into()),
                Value::Integer(ka_type_enum.into()),
            ),
            (
                Value::Integer(key_access_key::URL.into()),
                Value::Text(ka.url.clone()),
            ),
            (
                Value::Integer(key_access_key::PROTOCOL.into()),
                Value::Integer(protocol_enum.into()),
            ),
            (
                Value::Integer(key_access_key::WRAPPED_KEY.into()),
                Value::Bytes(wrapped_key_bytes),
            ),
            (
                Value::Integer(key_access_key::POLICY_BINDING.into()),
                Value::Map(policy_binding_map),
            ),
        ];

        // Add optional fields
        if let Some(ref kid) = ka.kid {
            ka_map.push((
                Value::Integer(key_access_key::KID.into()),
                Value::Text(kid.clone()),
            ));
        }

        if let Some(ref epk) = ka.ephemeral_public_key {
            let epk_bytes = BASE64.decode(epk).map_err(|e| {
                TdfCborError::EncodingError(format!("Invalid ephemeralPublicKey base64: {}", e))
            })?;
            ka_map.push((
                Value::Integer(key_access_key::EPHEMERAL_PUBLIC_KEY.into()),
                Value::Bytes(epk_bytes),
            ));
        }

        if let Some(ref sv) = ka.schema_version {
            ka_map.push((
                Value::Integer(key_access_key::SCHEMA_VERSION.into()),
                Value::Text(sv.clone()),
            ));
        }

        Ok(Value::Map(ka_map))
    }

    /// Encode method to CBOR
    #[cfg(feature = "cbor")]
    fn encode_method_to_cbor(
        &self,
        method: &EncryptionMethod,
    ) -> Result<ciborium::Value, TdfCborError> {
        use ciborium::Value;

        // Algorithm enum: "AES-256-GCM" -> 0
        let alg_enum = match method.algorithm.as_str() {
            "AES-256-GCM" => enums::SYMMETRIC_ALG_AES_256_GCM,
            _ => enums::SYMMETRIC_ALG_AES_256_GCM,
        };

        // Decode IV from base64 to raw bytes
        let iv_bytes = BASE64
            .decode(&method.iv)
            .map_err(|e| TdfCborError::EncodingError(format!("Invalid IV base64: {}", e)))?;

        let mut method_map = vec![
            (
                Value::Integer(method_key::ALGORITHM.into()),
                Value::Integer(alg_enum.into()),
            ),
            (
                Value::Integer(method_key::IV.into()),
                Value::Bytes(iv_bytes),
            ),
        ];

        method_map.push((
            Value::Integer(method_key::IS_STREAMABLE.into()),
            Value::Bool(method.is_streamable),
        ));

        Ok(Value::Map(method_map))
    }

    /// Encode integrity information to CBOR
    #[cfg(feature = "cbor")]
    fn encode_integrity_to_cbor(
        &self,
        integrity: &IntegrityInformation,
    ) -> Result<ciborium::Value, TdfCborError> {
        use ciborium::Value;

        // Encode root signature
        let root_sig = &integrity.root_signature;
        let root_alg_enum = self.hash_alg_to_enum(&root_sig.alg);
        let root_sig_bytes = BASE64.decode(&root_sig.sig).map_err(|e| {
            TdfCborError::EncodingError(format!("Invalid root signature base64: {}", e))
        })?;

        let root_sig_map = vec![
            (
                Value::Integer(root_sig_key::ALG.into()),
                Value::Integer(root_alg_enum.into()),
            ),
            (
                Value::Integer(root_sig_key::SIG.into()),
                Value::Bytes(root_sig_bytes),
            ),
        ];

        // Segment hash algorithm enum
        let seg_hash_alg_enum = self.hash_alg_to_enum(&integrity.segment_hash_alg);

        let mut integrity_map = vec![
            (
                Value::Integer(integrity_key::ROOT_SIGNATURE.into()),
                Value::Map(root_sig_map),
            ),
            (
                Value::Integer(integrity_key::SEGMENT_HASH_ALG.into()),
                Value::Integer(seg_hash_alg_enum.into()),
            ),
        ];

        // Encode segments
        if !integrity.segments.is_empty() {
            let segments_array: Vec<Value> = integrity
                .segments
                .iter()
                .map(|seg| self.encode_segment_to_cbor(seg))
                .collect::<Result<Vec<_>, _>>()?;
            integrity_map.push((
                Value::Integer(integrity_key::SEGMENTS.into()),
                Value::Array(segments_array),
            ));
        }

        integrity_map.push((
            Value::Integer(integrity_key::SEGMENT_SIZE_DEFAULT.into()),
            Value::Integer(integrity.segment_size_default.into()),
        ));

        integrity_map.push((
            Value::Integer(integrity_key::ENCRYPTED_SEGMENT_SIZE_DEFAULT.into()),
            Value::Integer(integrity.encrypted_segment_size_default.into()),
        ));

        Ok(Value::Map(integrity_map))
    }

    /// Encode a segment to CBOR
    #[cfg(feature = "cbor")]
    fn encode_segment_to_cbor(&self, segment: &Segment) -> Result<ciborium::Value, TdfCborError> {
        use ciborium::Value;

        let hash_bytes = BASE64.decode(&segment.hash).map_err(|e| {
            TdfCborError::EncodingError(format!("Invalid segment hash base64: {}", e))
        })?;

        let mut seg_map = vec![(
            Value::Integer(segment_key::HASH.into()),
            Value::Bytes(hash_bytes),
        )];

        if let Some(size) = segment.segment_size {
            seg_map.push((
                Value::Integer(segment_key::SEGMENT_SIZE.into()),
                Value::Integer(size.into()),
            ));
        }

        if let Some(size) = segment.encrypted_segment_size {
            seg_map.push((
                Value::Integer(segment_key::ENCRYPTED_SEGMENT_SIZE.into()),
                Value::Integer(size.into()),
            ));
        }

        Ok(Value::Map(seg_map))
    }

    /// Convert hash algorithm string to enum value
    fn hash_alg_to_enum(&self, alg: &str) -> u64 {
        match alg {
            "HS256" => enums::HASH_ALG_HS256,
            "HS384" => enums::HASH_ALG_HS384,
            "HS512" => enums::HASH_ALG_HS512,
            "GMAC" => enums::HASH_ALG_GMAC,
            "SHA256" => enums::HASH_ALG_SHA256,
            "ES256" => enums::HASH_ALG_ES256,
            "ES384" => enums::HASH_ALG_ES384,
            "ES512" => enums::HASH_ALG_ES512,
            _ => enums::HASH_ALG_HS256,
        }
    }

    /// Convert enum value to hash algorithm string
    fn enum_to_hash_alg(val: u64) -> String {
        match val {
            enums::HASH_ALG_HS256 => "HS256".to_string(),
            enums::HASH_ALG_HS384 => "HS384".to_string(),
            enums::HASH_ALG_HS512 => "HS512".to_string(),
            enums::HASH_ALG_GMAC => "GMAC".to_string(),
            enums::HASH_ALG_SHA256 => "SHA256".to_string(),
            enums::HASH_ALG_ES256 => "ES256".to_string(),
            enums::HASH_ALG_ES384 => "ES384".to_string(),
            enums::HASH_ALG_ES512 => "ES512".to_string(),
            _ => "HS256".to_string(),
        }
    }

    /// Decode manifest from native CBOR
    #[cfg(feature = "cbor")]
    fn decode_manifest_from_cbor(value: ciborium::Value) -> Result<TdfCborManifest, TdfCborError> {
        use ciborium::Value;

        let manifest_map = match value {
            Value::Map(m) => m,
            _ => {
                return Err(TdfCborError::DecodingError(
                    "Expected manifest map".to_string(),
                ));
            }
        };

        let mut enc_info = None;

        for (k, v) in manifest_map {
            let key = Self::extract_int_key(&k)?;

            if key == manifest_key::ENCRYPTION_INFORMATION {
                enc_info = Some(Self::decode_encryption_info(v)?);
            }
        }

        Ok(TdfCborManifest {
            encryption_information: enc_info
                .ok_or_else(|| TdfCborError::MissingField("encryptionInformation".to_string()))?,
            assertions: None, // TODO: decode assertions if present
        })
    }

    /// Decode encryption information from CBOR
    #[cfg(feature = "cbor")]
    fn decode_encryption_info(
        value: ciborium::Value,
    ) -> Result<EncryptionInformation, TdfCborError> {
        use ciborium::Value;

        let enc_map = match value {
            Value::Map(m) => m,
            _ => {
                return Err(TdfCborError::DecodingError(
                    "Expected encryptionInformation map".to_string(),
                ));
            }
        };

        let mut enc_type = "split".to_string();
        let mut key_access = Vec::new();
        let mut method = None;
        let mut integrity = None;
        let mut policy = String::new();

        for (k, v) in enc_map {
            let key = Self::extract_int_key(&k)?;

            match key {
                enc_info_key::TYPE => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        enc_type = match i128_val as u64 {
                            enums::ENCRYPTION_TYPE_SPLIT => "split".to_string(),
                            _ => "split".to_string(),
                        };
                    }
                }
                enc_info_key::KEY_ACCESS => {
                    if let Value::Array(arr) = v {
                        for ka_val in arr {
                            key_access.push(Self::decode_key_access(ka_val)?);
                        }
                    }
                }
                enc_info_key::METHOD => {
                    method = Some(Self::decode_method(v)?);
                }
                enc_info_key::INTEGRITY_INFORMATION => {
                    integrity = Some(Self::decode_integrity(v)?);
                }
                enc_info_key::POLICY => {
                    // Policy is stored as raw bytes, encode to base64
                    if let Value::Bytes(b) = v {
                        policy = BASE64.encode(&b);
                    }
                }
                _ => {}
            }
        }

        Ok(EncryptionInformation {
            encryption_type: enc_type,
            key_access,
            method: method.ok_or_else(|| TdfCborError::MissingField("method".to_string()))?,
            integrity_information: integrity
                .ok_or_else(|| TdfCborError::MissingField("integrityInformation".to_string()))?,
            policy,
        })
    }

    /// Decode key access from CBOR
    #[cfg(feature = "cbor")]
    fn decode_key_access(value: ciborium::Value) -> Result<KeyAccess, TdfCborError> {
        use ciborium::Value;

        let ka_map = match value {
            Value::Map(m) => m,
            _ => {
                return Err(TdfCborError::DecodingError(
                    "Expected keyAccess map".to_string(),
                ));
            }
        };

        let mut access_type = "wrapped".to_string();
        let mut url = String::new();
        let mut protocol = "kas".to_string();
        let mut wrapped_key = String::new();
        let mut policy_binding = None;
        let mut kid = None;
        let mut ephemeral_public_key = None;
        let mut schema_version = None;

        for (k, v) in ka_map {
            let key = Self::extract_int_key(&k)?;

            match key {
                key_access_key::TYPE => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        access_type = match i128_val as u64 {
                            enums::KEY_ACCESS_TYPE_WRAPPED => "wrapped".to_string(),
                            enums::KEY_ACCESS_TYPE_REMOTE => "remote".to_string(),
                            _ => "wrapped".to_string(),
                        };
                    }
                }
                key_access_key::URL => {
                    if let Value::Text(s) = v {
                        url = s;
                    }
                }
                key_access_key::PROTOCOL => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        protocol = match i128_val as u64 {
                            enums::KEY_PROTOCOL_KAS => "kas".to_string(),
                            _ => "kas".to_string(),
                        };
                    }
                }
                key_access_key::WRAPPED_KEY => {
                    if let Value::Bytes(b) = v {
                        wrapped_key = BASE64.encode(&b);
                    }
                }
                key_access_key::POLICY_BINDING => {
                    policy_binding = Some(Self::decode_policy_binding(v)?);
                }
                key_access_key::KID => {
                    if let Value::Text(s) = v {
                        kid = Some(s);
                    }
                }
                key_access_key::EPHEMERAL_PUBLIC_KEY => {
                    if let Value::Bytes(b) = v {
                        ephemeral_public_key = Some(BASE64.encode(&b));
                    }
                }
                key_access_key::SCHEMA_VERSION => {
                    if let Value::Text(s) = v {
                        schema_version = Some(s);
                    }
                }
                _ => {}
            }
        }

        Ok(KeyAccess {
            access_type,
            url,
            protocol,
            wrapped_key,
            policy_binding: policy_binding
                .ok_or_else(|| TdfCborError::MissingField("policyBinding".to_string()))?,
            encrypted_metadata: None,
            kid,
            ephemeral_public_key,
            schema_version,
        })
    }

    /// Decode policy binding from CBOR
    #[cfg(feature = "cbor")]
    fn decode_policy_binding(
        value: ciborium::Value,
    ) -> Result<crate::manifest::PolicyBinding, TdfCborError> {
        use ciborium::Value;

        let pb_map = match value {
            Value::Map(m) => m,
            _ => {
                return Err(TdfCborError::DecodingError(
                    "Expected policyBinding map".to_string(),
                ));
            }
        };

        let mut alg = "HS256".to_string();
        let mut hash = String::new();

        for (k, v) in pb_map {
            let key = Self::extract_int_key(&k)?;

            match key {
                policy_binding_key::ALG => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        alg = Self::enum_to_hash_alg(i128_val as u64);
                    }
                }
                policy_binding_key::HASH => {
                    if let Value::Bytes(b) = v {
                        hash = BASE64.encode(&b);
                    }
                }
                _ => {}
            }
        }

        Ok(crate::manifest::PolicyBinding { alg, hash })
    }

    /// Decode method from CBOR
    #[cfg(feature = "cbor")]
    fn decode_method(value: ciborium::Value) -> Result<EncryptionMethod, TdfCborError> {
        use ciborium::Value;

        let method_map = match value {
            Value::Map(m) => m,
            _ => {
                return Err(TdfCborError::DecodingError(
                    "Expected method map".to_string(),
                ));
            }
        };

        let mut algorithm = "AES-256-GCM".to_string();
        let mut iv = String::new();
        let mut is_streamable = true; // Default to true

        for (k, v) in method_map {
            let key = Self::extract_int_key(&k)?;

            match key {
                method_key::ALGORITHM => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        algorithm = match i128_val as u64 {
                            enums::SYMMETRIC_ALG_AES_256_GCM => "AES-256-GCM".to_string(),
                            _ => "AES-256-GCM".to_string(),
                        };
                    }
                }
                method_key::IV => {
                    if let Value::Bytes(b) = v {
                        iv = BASE64.encode(&b);
                    }
                }
                method_key::IS_STREAMABLE => {
                    if let Value::Bool(b) = v {
                        is_streamable = b;
                    }
                }
                _ => {}
            }
        }

        Ok(EncryptionMethod {
            algorithm,
            iv,
            is_streamable,
        })
    }

    /// Decode integrity information from CBOR
    #[cfg(feature = "cbor")]
    fn decode_integrity(value: ciborium::Value) -> Result<IntegrityInformation, TdfCborError> {
        use ciborium::Value;

        let int_map = match value {
            Value::Map(m) => m,
            _ => {
                return Err(TdfCborError::DecodingError(
                    "Expected integrityInformation map".to_string(),
                ));
            }
        };

        let mut root_signature = None;
        let mut segment_hash_alg = "GMAC".to_string();
        let mut segments = Vec::new();
        let mut segment_size_default: u64 = 0;
        let mut encrypted_segment_size_default: u64 = 0;

        for (k, v) in int_map {
            let key = Self::extract_int_key(&k)?;

            match key {
                integrity_key::ROOT_SIGNATURE => {
                    root_signature = Some(Self::decode_root_signature(v)?);
                }
                integrity_key::SEGMENT_HASH_ALG => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        segment_hash_alg = Self::enum_to_hash_alg(i128_val as u64);
                    }
                }
                integrity_key::SEGMENTS => {
                    if let Value::Array(arr) = v {
                        for seg_val in arr {
                            segments.push(Self::decode_segment(seg_val)?);
                        }
                    }
                }
                integrity_key::SEGMENT_SIZE_DEFAULT => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        segment_size_default = i128_val as u64;
                    }
                }
                integrity_key::ENCRYPTED_SEGMENT_SIZE_DEFAULT => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        encrypted_segment_size_default = i128_val as u64;
                    }
                }
                _ => {}
            }
        }

        Ok(IntegrityInformation {
            root_signature: root_signature
                .ok_or_else(|| TdfCborError::MissingField("rootSignature".to_string()))?,
            segment_hash_alg,
            segments,
            segment_size_default,
            encrypted_segment_size_default,
        })
    }

    /// Decode root signature from CBOR
    #[cfg(feature = "cbor")]
    fn decode_root_signature(value: ciborium::Value) -> Result<RootSignature, TdfCborError> {
        use ciborium::Value;

        let sig_map = match value {
            Value::Map(m) => m,
            _ => {
                return Err(TdfCborError::DecodingError(
                    "Expected rootSignature map".to_string(),
                ));
            }
        };

        let mut alg = "HS256".to_string();
        let mut sig = String::new();

        for (k, v) in sig_map {
            let key = Self::extract_int_key(&k)?;

            match key {
                root_sig_key::ALG => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        alg = Self::enum_to_hash_alg(i128_val as u64);
                    }
                }
                root_sig_key::SIG => {
                    if let Value::Bytes(b) = v {
                        sig = BASE64.encode(&b);
                    }
                }
                _ => {}
            }
        }

        Ok(RootSignature { alg, sig })
    }

    /// Decode segment from CBOR
    #[cfg(feature = "cbor")]
    fn decode_segment(value: ciborium::Value) -> Result<Segment, TdfCborError> {
        use ciborium::Value;

        let seg_map = match value {
            Value::Map(m) => m,
            _ => {
                return Err(TdfCborError::DecodingError(
                    "Expected segment map".to_string(),
                ));
            }
        };

        let mut hash = String::new();
        let mut segment_size = None;
        let mut encrypted_segment_size = None;

        for (k, v) in seg_map {
            let key = Self::extract_int_key(&k)?;

            match key {
                segment_key::HASH => {
                    if let Value::Bytes(b) = v {
                        hash = BASE64.encode(&b);
                    }
                }
                segment_key::SEGMENT_SIZE => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        segment_size = Some(i128_val as u64);
                    }
                }
                segment_key::ENCRYPTED_SEGMENT_SIZE => {
                    if let Value::Integer(i) = v {
                        let i128_val: i128 = i.into();
                        encrypted_segment_size = Some(i128_val as u64);
                    }
                }
                _ => {}
            }
        }

        Ok(Segment {
            hash,
            segment_size,
            encrypted_segment_size,
        })
    }

    /// Extract integer key from CBOR value
    #[cfg(feature = "cbor")]
    fn extract_int_key(value: &ciborium::Value) -> Result<u64, TdfCborError> {
        use ciborium::Value;

        match value {
            Value::Integer(i) => {
                let i128_val: i128 = (*i).into();
                Ok(i128_val as u64)
            }
            _ => Err(TdfCborError::DecodingError(
                "Expected integer key".to_string(),
            )),
        }
    }

    /// Decrypt the payload with a provided data encryption key
    pub fn decrypt_with_key(&self, payload_key: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        use aes_gcm::{
            Aes256Gcm, Nonce,
            aead::{Aead, KeyInit},
        };

        let ciphertext = &self.payload.value;

        // Extract IV from encryption method
        let iv_bytes = BASE64.decode(&self.manifest.encryption_information.method.iv)?;

        let payload_iv = if iv_bytes.len() >= 12 {
            &iv_bytes[0..12]
        } else {
            &iv_bytes[..]
        };

        let cipher = Aes256Gcm::new_from_slice(payload_key)
            .map_err(|_| EncryptionError::InvalidKeyLength)?;

        #[allow(deprecated)]
        let nonce = Nonce::from_slice(payload_iv);

        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(EncryptionError::AeadError)?;

        Ok(plaintext)
    }
}

#[cfg(feature = "kas-client")]
impl TdfCborBuilder {
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
    /// This method is required - TDF-CBOR documents cannot be created without
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

    /// Build the TDF-CBOR container
    pub fn build(self) -> Result<TdfCbor, TdfCborError> {
        let kas_url = self
            .kas_url
            .ok_or_else(|| TdfCborError::MissingField("kas_url".to_string()))?;
        let policy = self
            .policy
            .ok_or_else(|| TdfCborError::MissingField("policy".to_string()))?;

        // Create encryption instance
        let tdf_encryption = TdfEncryption::new()?;

        // Encrypt the data
        let encrypted_payload = tdf_encryption.encrypt(&self.data)?;

        // Get the payload key for policy binding
        let payload_key = tdf_encryption.payload_key();

        // Create policy binding
        let policy_json = serde_json::to_string(&policy)
            .map_err(|e| TdfCborError::EncodingError(e.to_string()))?;
        let policy_b64 = BASE64.encode(policy_json.as_bytes());

        // Calculate policy binding hash
        let policy_hash = calculate_policy_binding(&policy_b64, payload_key)
            .map_err(|e| TdfCborError::EncodingError(e.to_string()))?;

        // Decode the base64 ciphertext to get raw bytes
        let ciphertext_bytes = BASE64.decode(&encrypted_payload.ciphertext)?;

        // Wrap key using EC (ECDH + HKDF + AES-GCM) - KAS public key is required
        // This requires the kas-client feature which provides EC key wrapping
        let kas_pem = self
            .kas_public_key_pem
            .ok_or_else(|| TdfCborError::MissingField("kas_public_key".to_string()))?;
        let ec_result = opentdf_crypto::wrap_key_with_ec(&kas_pem, payload_key)
            .map_err(|e| TdfCborError::EncodingError(format!("EC wrap failed: {:?}", e)))?;
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

        // Extract GMAC tag from encrypted payload (last 16 bytes)
        let gmac_tag = if ciphertext_bytes.len() >= 16 {
            ciphertext_bytes[ciphertext_bytes.len() - 16..].to_vec()
        } else {
            return Err(TdfCborError::EncodingError(
                "Ciphertext too short for GMAC tag".to_string(),
            ));
        };

        // Create integrity information
        let mut integrity_info = IntegrityInformation {
            root_signature: RootSignature {
                alg: "HS256".to_string(),
                sig: String::new(),
            },
            segment_hash_alg: "GMAC".to_string(),
            segments: vec![Segment {
                hash: BASE64.encode(&gmac_tag),
                segment_size: Some(self.data.len() as u64),
                encrypted_segment_size: Some(ciphertext_bytes.len() as u64),
            }],
            segment_size_default: self.data.len() as u64,
            encrypted_segment_size_default: ciphertext_bytes.len() as u64,
        };

        // Calculate root signature
        integrity_info
            .generate_root_signature(&[gmac_tag], payload_key)
            .map_err(TdfCborError::EncodingError)?;

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

        // Create manifest
        let manifest = TdfCborManifest {
            encryption_information: encryption_info,
            assertions: None,
        };

        // Create payload with raw binary (not base64)
        let payload = CborPayload {
            payload_type: "inline".to_string(),
            protocol: "binary".to_string(), // Key difference from TDF-JSON
            mime_type: self.mime_type,
            is_encrypted: true,
            value: ciphertext_bytes,
        };

        // Create timestamp
        let created = if self.include_created {
            Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            )
        } else {
            None
        };

        Ok(TdfCbor {
            tdf: "cbor".to_string(),
            version: [1, 0, 0],
            created,
            manifest,
            payload,
        })
    }
}

// ============================================================================
// Format Detection
// ============================================================================

/// Detect if data is a TDF-CBOR document
pub fn is_tdf_cbor(data: &[u8]) -> bool {
    TdfCbor::has_magic_bytes(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Get test KAS URL from environment variable or use default
    fn test_kas_url() -> String {
        std::env::var("TEST_KAS_URL").unwrap_or_else(|_| "https://100.arkavo.net".to_string())
    }

    /// Generate a test EC key pair for testing TDF-CBOR encryption
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
    fn test_cbor_magic_bytes() {
        assert_eq!(CBOR_MAGIC, [0xD9, 0xD9, 0xF7, 0xA5]);
    }

    #[test]
    fn test_has_magic_bytes() {
        // Valid self-describe tag + any map
        let valid = [0xD9, 0xD9, 0xF7, 0xA5, 0x01, 0x02];
        assert!(TdfCbor::has_magic_bytes(&valid));

        // Valid self-describe tag + different map size
        let valid2 = [0xD9, 0xD9, 0xF7, 0xB8, 0x05];
        assert!(TdfCbor::has_magic_bytes(&valid2));

        let invalid = [0x00, 0x01, 0x02, 0x03];
        assert!(!TdfCbor::has_magic_bytes(&invalid));

        let short = [0xD9, 0xD9];
        assert!(!TdfCbor::has_magic_bytes(&short));
    }

    #[cfg(feature = "kas-client")]
    #[test]
    fn test_create_cbor_envelope() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );
        let (_private_pem, public_pem) = generate_test_ec_key_pair();

        let container = TdfCbor::encrypt(b"Hello, World!")
            .kas_url(&test_kas_url())
            .kas_public_key(&public_pem)
            .policy(policy)
            .mime_type("text/plain")
            .build()
            .expect("Failed to create container");

        assert_eq!(container.tdf, "cbor");
        assert_eq!(container.version, [1, 0, 0]);
        assert!(container.created.is_some());
        assert_eq!(container.payload.payload_type, "inline");
        assert_eq!(container.payload.protocol, "binary");
        assert_eq!(container.payload.mime_type, Some("text/plain".to_string()));
        assert!(container.payload.is_encrypted);
        assert!(!container.payload.value.is_empty());
    }

    #[cfg(all(feature = "cbor", feature = "kas-client"))]
    #[test]
    fn test_cbor_roundtrip() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );
        let (_private_pem, public_pem) = generate_test_ec_key_pair();

        let original = TdfCbor::encrypt(b"Test data for CBOR")
            .kas_url(&test_kas_url())
            .kas_public_key(&public_pem)
            .policy(policy)
            .include_created(false)
            .build()
            .expect("Failed to create container");

        // Serialize to bytes
        let bytes = original.to_bytes().expect("Failed to serialize");

        // Verify magic bytes
        assert!(TdfCbor::has_magic_bytes(&bytes));

        // Parse back
        let parsed = TdfCbor::from_bytes(&bytes).expect("Failed to parse");

        assert_eq!(parsed.tdf, "cbor");
        assert_eq!(parsed.version, [1, 0, 0]);
        assert_eq!(parsed.payload.value, original.payload.value);
    }

    #[test]
    fn test_builder_missing_kas_url() {
        let policy = Policy::new(
            "test-policy".to_string(),
            vec![],
            vec!["user@example.com".to_string()],
        );

        let result = TdfCbor::encrypt(b"Data").policy(policy).build();

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_missing_policy() {
        let result = TdfCbor::encrypt(b"Data").kas_url(&test_kas_url()).build();

        assert!(result.is_err());
    }
}
