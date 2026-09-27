//! ZTDF-JSON types for inline TDF payloads in JSON-RPC.
//!
//! These types follow the TDF3 manifest structure but adapted for
//! inline JSON-RPC transmission (no ZIP overhead).

use serde::{Deserialize, Serialize};

/// TDF manifest for JSON-RPC inline format (ZTDF-JSON).
///
/// Adapts TDF3 manifest structure for JSON-RPC protocols where
/// the encrypted payload is inlined as base64.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TdfManifest {
    /// Encryption metadata including key access and method
    #[serde(rename = "encryptionInformation")]
    pub encryption_information: EncryptionInformation,

    /// The encrypted payload (base64-encoded ciphertext)
    pub payload: InlinePayload,

    /// TDF version (e.g., "3.0.0")
    pub version: String,
}

impl TdfManifest {
    /// Create a new TDF manifest with the given components.
    #[must_use]
    pub fn new(encryption_information: EncryptionInformation, payload: InlinePayload) -> Self {
        Self {
            encryption_information,
            payload,
            version: "3.0.0".to_string(),
        }
    }
}

/// Encryption metadata including key access objects and algorithm.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncryptionInformation {
    /// Key split type (typically "split")
    #[serde(rename = "type")]
    pub key_type: String,

    /// Key access objects for each KAS
    #[serde(rename = "keyAccess")]
    pub key_access: Vec<KeyAccessObject>,

    /// Encryption method and parameters
    pub method: EncryptionMethod,

    /// Base64-encoded policy JSON
    pub policy: String,
}

impl Default for EncryptionInformation {
    fn default() -> Self {
        Self {
            key_type: "split".to_string(),
            key_access: Vec::new(),
            method: EncryptionMethod::default(),
            policy: String::new(),
        }
    }
}

/// Key access object describing how to retrieve the decryption key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyAccessObject {
    /// Access type (typically "wrapped")
    #[serde(rename = "type")]
    pub access_type: String,

    /// KAS URL for key retrieval
    pub url: String,

    /// Protocol (typically "kas")
    pub protocol: String,

    /// The wrapped (encrypted) key material
    #[serde(rename = "wrappedKey")]
    pub wrapped_key: String,

    /// Cryptographic binding of policy to key
    #[serde(rename = "policyBinding")]
    pub policy_binding: PolicyBinding,
}

impl KeyAccessObject {
    /// Create a new key access object for a KAS.
    #[must_use]
    pub fn new(kas_url: &str, wrapped_key: &str, policy_binding: PolicyBinding) -> Self {
        Self {
            access_type: "wrapped".to_string(),
            url: kas_url.to_string(),
            protocol: "kas".to_string(),
            wrapped_key: wrapped_key.to_string(),
            policy_binding,
        }
    }
}

/// Cryptographic binding of policy to wrapped key.
///
/// Ensures the policy cannot be modified without invalidating the key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyBinding {
    /// HMAC algorithm (typically "HS256")
    pub alg: String,

    /// HMAC of policy bound to key material
    pub hash: String,
}

impl PolicyBinding {
    /// Create a new policy binding with HMAC-SHA256.
    #[must_use]
    pub fn new(hash: &str) -> Self {
        Self {
            alg: "HS256".to_string(),
            hash: hash.to_string(),
        }
    }
}

impl Default for PolicyBinding {
    fn default() -> Self {
        Self {
            alg: "HS256".to_string(),
            hash: String::new(),
        }
    }
}

/// Encryption method and algorithm parameters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncryptionMethod {
    /// Algorithm name (e.g., "AES-256-GCM")
    pub algorithm: String,

    /// Initialization vector (base64-encoded)
    pub iv: String,

    /// Whether the encryption supports streaming
    #[serde(rename = "isStreamable")]
    pub is_streamable: bool,
}

impl Default for EncryptionMethod {
    fn default() -> Self {
        Self {
            algorithm: "AES-256-GCM".to_string(),
            iv: String::new(),
            is_streamable: true,
        }
    }
}

/// Inline payload for ZTDF-JSON format.
///
/// Contains the encrypted ciphertext encoded as base64 for JSON transport.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InlinePayload {
    /// Payload type (always "inline" for ZTDF-JSON)
    #[serde(rename = "type")]
    pub payload_type: String,

    /// MIME type of the original content
    #[serde(rename = "mimeType")]
    pub mime_type: String,

    /// Encoding protocol (typically "base64")
    pub protocol: String,

    /// Base64-encoded ciphertext
    pub value: String,
}

impl InlinePayload {
    /// Create a new inline payload with the given ciphertext.
    #[must_use]
    pub fn new(ciphertext_base64: &str, mime_type: &str) -> Self {
        Self {
            payload_type: "inline".to_string(),
            mime_type: mime_type.to_string(),
            protocol: "base64".to_string(),
            value: ciphertext_base64.to_string(),
        }
    }

    /// Create a payload for application/octet-stream content.
    #[must_use]
    pub fn binary(ciphertext_base64: &str) -> Self {
        Self::new(ciphertext_base64, "application/octet-stream")
    }

    /// Create a payload for application/json content.
    #[must_use]
    pub fn json(ciphertext_base64: &str) -> Self {
        Self::new(ciphertext_base64, "application/json")
    }
}

impl Default for InlinePayload {
    fn default() -> Self {
        Self {
            payload_type: "inline".to_string(),
            mime_type: "application/octet-stream".to_string(),
            protocol: "base64".to_string(),
            value: String::new(),
        }
    }
}

/// Policy for TDF encryption with attribute-based access control.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Policy {
    /// Unique policy identifier (UUID)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Required attributes for access
    pub attributes: Vec<Attribute>,

    /// Dissemination controls (e.g., country codes)
    pub dissemination: Vec<String>,
}

/// Attribute requirement for policy evaluation.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Attribute {
    /// Fully qualified attribute name (FQN)
    /// e.g., "https://arkavo.net/attr/role"
    pub attribute: String,

    /// Required values for this attribute
    pub values: Vec<String>,
}

impl<'de> Deserialize<'de> for Attribute {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AttributeRaw {
            attribute: String,
            values: Vec<String>,
        }
        let raw = AttributeRaw::deserialize(deserializer)?;
        if raw.attribute.is_empty() {
            return Err(serde::de::Error::custom("attribute FQN cannot be empty"));
        }
        Ok(Attribute {
            attribute: raw.attribute,
            values: raw.values,
        })
    }
}

impl Attribute {
    /// Create a new attribute requirement.
    #[must_use]
    pub fn new(fqn: &str, values: &[&str]) -> Self {
        Self {
            attribute: fqn.to_string(),
            values: values.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("TDFS-003")]
    #[test]
    fn tdf_manifest_serialization() {
        let manifest = TdfManifest {
            encryption_information: EncryptionInformation {
                key_type: "split".to_string(),
                key_access: vec![KeyAccessObject {
                    access_type: "wrapped".to_string(),
                    url: "https://kas.example.com".to_string(),
                    protocol: "kas".to_string(),
                    wrapped_key: "base64_wrapped_key".to_string(),
                    policy_binding: PolicyBinding {
                        alg: "HS256".to_string(),
                        hash: "policy_hash".to_string(),
                    },
                }],
                method: EncryptionMethod {
                    algorithm: "AES-256-GCM".to_string(),
                    iv: "base64_iv".to_string(),
                    is_streamable: true,
                },
                policy: "base64_policy".to_string(),
            },
            payload: InlinePayload {
                payload_type: "inline".to_string(),
                mime_type: "application/octet-stream".to_string(),
                protocol: "base64".to_string(),
                value: "encrypted_data".to_string(),
            },
            version: "3.0.0".to_string(),
        };

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        assert!(json.contains("encryptionInformation"));
        assert!(json.contains("keyAccess"));
        assert!(json.contains("wrappedKey"));
        assert!(json.contains("policyBinding"));

        let parsed: TdfManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, manifest);
    }

    #[spec("TDFS-003")]
    #[test]
    fn policy_serialization() {
        let policy = Policy {
            id: Some("test-uuid".to_string()),
            attributes: vec![Attribute::new(
                "https://arkavo.net/attr/role",
                &["admin", "user"],
            )],
            dissemination: vec!["US".to_string(), "CA".to_string()],
        };

        let json = serde_json::to_string(&policy).unwrap();
        let parsed: Policy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, policy);
    }

    #[spec("TDFS-003")]
    #[test]
    fn inline_payload_constructors() {
        let binary = InlinePayload::binary("Y2lwaGVydGV4dA==");
        assert_eq!(binary.mime_type, "application/octet-stream");
        assert_eq!(binary.payload_type, "inline");

        let json = InlinePayload::json("eyJrZXkiOiJ2YWx1ZSJ9");
        assert_eq!(json.mime_type, "application/json");
    }

    #[spec("TDFS-003")]
    #[spec("TDFS-011")]
    #[test]
    fn key_access_object_constructor() {
        let kao = KeyAccessObject::new(
            "https://platform.arkavo.net",
            "wrapped_key_base64",
            PolicyBinding::new("hash_value"),
        );

        assert_eq!(kao.access_type, "wrapped");
        assert_eq!(kao.protocol, "kas");
        assert_eq!(kao.url, "https://platform.arkavo.net");
    }

    #[spec("TDFS-008")]
    #[test]
    fn attribute_constructor() {
        let attr = Attribute::new(
            "https://arkavo.net/attr/clearance",
            &["secret", "top-secret"],
        );
        assert_eq!(attr.attribute, "https://arkavo.net/attr/clearance");
        assert_eq!(attr.values.len(), 2);
    }

    #[spec("TDFS-003")]
    #[test]
    fn default_implementations() {
        let info = EncryptionInformation::default();
        assert_eq!(info.key_type, "split");
        assert!(info.key_access.is_empty());

        let method = EncryptionMethod::default();
        assert_eq!(method.algorithm, "AES-256-GCM");
        assert!(method.is_streamable);

        let payload = InlinePayload::default();
        assert_eq!(payload.payload_type, "inline");
        assert_eq!(payload.protocol, "base64");

        let binding = PolicyBinding::default();
        assert_eq!(binding.alg, "HS256");
    }
}
