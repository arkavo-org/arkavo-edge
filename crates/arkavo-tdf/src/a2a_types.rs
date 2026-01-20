//! A2A KAS request/response types for JSON-RPC.
//!
//! These types define the KAS capability interface for the A2A protocol,
//! enabling agents to perform TDF key rewrap operations via JSON-RPC.

use serde::{Deserialize, Serialize};

use crate::types::PolicyBinding;

/// Request to rewrap a TDF encryption key for a specific client.
///
/// The wrapped key is decrypted by KAS and re-encrypted for the client's
/// public key, after verifying the caller has access via NTDF delegation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KasRewrapRequest {
    /// Base64-encoded wrapped key from the TDF manifest
    pub wrapped_key: String,

    /// Policy binding from the TDF manifest (HMAC verification)
    pub policy_binding: PolicyBinding,

    /// Base64-encoded policy JSON from the TDF manifest
    pub policy: String,

    /// NTDF delegation token chain proving access entitlements
    pub delegation_token: String,

    /// Client's public key in PEM format for rewrapping
    pub client_public_key: String,
}

/// Response containing the rewrapped key for the client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KasRewrapResponse {
    /// Key rewrapped (encrypted) for the client's public key
    pub entity_wrapped_key: String,
}

/// Request to retrieve the KAS public key for TDF encryption.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct KasPublicKeyRequest {
    /// Requested algorithm (e.g., "RSA-OAEP"). If not specified, returns default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
}

/// Response containing the KAS public key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KasPublicKeyResponse {
    /// PEM-encoded public key
    pub public_key: String,

    /// Key identifier for key rotation tracking
    pub key_id: String,

    /// Algorithm this key supports (e.g., "RSA-OAEP")
    pub algorithm: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kas_rewrap_request_serialization() {
        let request = KasRewrapRequest {
            wrapped_key: "dGVzdC13cmFwcGVkLWtleQ==".to_string(),
            policy_binding: PolicyBinding::new("test-hash"),
            policy: "eyJhdHRyaWJ1dGVzIjpbXX0=".to_string(),
            delegation_token: "ntdf-token-chain".to_string(),
            client_public_key: "-----BEGIN PUBLIC KEY-----\nMIIB...\n-----END PUBLIC KEY-----"
                .to_string(),
        };

        let json = serde_json::to_string(&request).unwrap();
        let parsed: KasRewrapRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
    }

    #[test]
    fn kas_rewrap_response_serialization() {
        let response = KasRewrapResponse {
            entity_wrapped_key: "cmV3cmFwcGVkLWZvci1jbGllbnQ=".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: KasRewrapResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, response);
    }

    #[test]
    fn kas_public_key_request_serialization() {
        let request_with_alg = KasPublicKeyRequest {
            algorithm: Some("RSA-OAEP".to_string()),
        };
        let json = serde_json::to_string(&request_with_alg).unwrap();
        assert!(json.contains("RSA-OAEP"));

        let request_default = KasPublicKeyRequest::default();
        let json = serde_json::to_string(&request_default).unwrap();
        assert!(!json.contains("algorithm"));
    }

    #[test]
    fn kas_public_key_response_serialization() {
        let response = KasPublicKeyResponse {
            public_key: "-----BEGIN PUBLIC KEY-----\nMIIB...\n-----END PUBLIC KEY-----".to_string(),
            key_id: "kas-key-1".to_string(),
            algorithm: "RSA-OAEP".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: KasPublicKeyResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, response);
    }
}
