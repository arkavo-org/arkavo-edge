//! Request and response types for KAS rewrap operations

use serde::{Deserialize, Serialize};

/// Algorithm type for rewrap operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAlgorithm {
    /// EC P-256 for NanoTDF
    EcP256,
    /// RSA-2048 for Standard TDF
    Rsa2048,
}

impl KeyAlgorithm {
    /// Parse algorithm string from OpenTDF spec format
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "" => Some(KeyAlgorithm::Rsa2048), // Empty = Standard TDF RSA
            s if s.starts_with("ec") || s.contains("secp256") => Some(KeyAlgorithm::EcP256),
            s if s.starts_with("rsa") => Some(KeyAlgorithm::Rsa2048),
            _ => None,
        }
    }

    /// Check if this is an EC algorithm
    pub fn is_ec(&self) -> bool {
        matches!(self, KeyAlgorithm::EcP256)
    }

    /// Check if this is an RSA algorithm
    pub fn is_rsa(&self) -> bool {
        matches!(self, KeyAlgorithm::Rsa2048)
    }
}

/// Key access object for rewrap
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyAccessObject {
    /// Base64-encoded header (NanoTDF header or RSA-wrapped DEK)
    pub header: String,
    /// Key access type
    #[serde(rename = "type")]
    pub type_field: String,
    /// KAS URL
    pub url: String,
    /// Protocol (e.g., "nanotdf", "tdf")
    pub protocol: String,
}

/// Information about a key to unwrap
#[derive(Debug, Clone)]
pub struct KeyInfo {
    /// Algorithm type
    pub algorithm: KeyAlgorithm,
    /// Base64-encoded wrapped key material
    pub wrapped_key: String,
    /// Optional policy ID
    pub policy_id: Option<String>,
}

/// Result of a rewrap operation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewrapResult {
    /// Status: "permit" or "fail"
    pub status: String,
    /// Base64-encoded rewrapped key (nonce + ciphertext + tag)
    pub kas_wrapped_key: Option<String>,
    /// Error message if status is "fail"
    pub error: Option<String>,
}

impl RewrapResult {
    /// Create a successful result
    pub fn success(wrapped_key: String) -> Self {
        Self {
            status: "permit".to_string(),
            kas_wrapped_key: Some(wrapped_key),
            error: None,
        }
    }

    /// Create a failed result
    pub fn failure(error: String) -> Self {
        Self {
            status: "fail".to_string(),
            kas_wrapped_key: None,
            error: Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_algorithm_parsing() {
        assert_eq!(
            KeyAlgorithm::parse("ec:secp256r1"),
            Some(KeyAlgorithm::EcP256)
        );
        assert_eq!(KeyAlgorithm::parse("ec"), Some(KeyAlgorithm::EcP256));
        assert_eq!(KeyAlgorithm::parse("rsa:2048"), Some(KeyAlgorithm::Rsa2048));
        assert_eq!(KeyAlgorithm::parse("rsa"), Some(KeyAlgorithm::Rsa2048));
        assert_eq!(KeyAlgorithm::parse(""), Some(KeyAlgorithm::Rsa2048));
        assert_eq!(KeyAlgorithm::parse("unknown"), None);
    }

    #[test]
    fn test_rewrap_result() {
        let success = RewrapResult::success("encoded_key".to_string());
        assert_eq!(success.status, "permit");
        assert!(success.kas_wrapped_key.is_some());
        assert!(success.error.is_none());

        let failure = RewrapResult::failure("error message".to_string());
        assert_eq!(failure.status, "fail");
        assert!(failure.kas_wrapped_key.is_none());
        assert!(failure.error.is_some());
    }
}
