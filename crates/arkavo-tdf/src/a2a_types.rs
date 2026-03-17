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

// --- TDF P2P Share types ---

/// Request to share TDF-encrypted data with another agent via Iroh P2P.
///
/// Sent from the encrypting agent to the target agent via `tdf.share` RPC.
/// The target agent stores this as a pending offer and can later fetch + decrypt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TdfShareRequest {
    /// Iroh blob ticket for fetching the encrypted data
    pub ticket: String,

    /// BLAKE3 content hash of the encrypted blob
    pub content_hash: String,

    /// Size of the encrypted blob in bytes
    pub size_bytes: u64,

    /// Policy attribute namespaces applied to the TDF
    pub policy_attributes: Vec<String>,

    /// KAS URL that can rewrap the encryption key
    pub kas_url: String,

    /// Agent ID of the sender
    pub sender_agent_id: String,
}

/// Response to a `tdf.share` request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TdfShareResponse {
    /// Whether the agent accepted the share offer
    pub accepted: bool,

    /// Unique identifier for this offer (for later retrieval)
    pub offer_id: String,
}

/// Request to list pending TDF share offers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TdfOffersRequest {}

/// A pending TDF share offer from another agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TdfOffer {
    /// Unique offer identifier
    pub offer_id: String,

    /// Iroh blob ticket for fetching the encrypted data
    pub ticket: String,

    /// BLAKE3 content hash of the encrypted blob
    pub content_hash: String,

    /// Size of the encrypted blob in bytes
    pub size_bytes: u64,

    /// Policy attribute namespaces applied to the TDF
    pub policy_attributes: Vec<String>,

    /// KAS URL that can rewrap the encryption key
    pub kas_url: String,

    /// Agent ID of the sender
    pub sender_agent_id: String,

    /// ISO 8601 timestamp when the offer was received
    pub received_at: String,
}

/// Response containing pending TDF share offers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TdfOffersResponse {
    /// List of pending offers
    pub offers: Vec<TdfOffer>,
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

    #[test]
    fn tdf_share_request_serialization() {
        let request = TdfShareRequest {
            ticket: "blobaaaa...".to_string(),
            content_hash: "abc123".to_string(),
            size_bytes: 1024,
            policy_attributes: vec!["https://arkavo.net/attr/sensitivity".to_string()],
            kas_url: "https://kas.arkavo.net".to_string(),
            sender_agent_id: "agent-001".to_string(),
        };

        let json = serde_json::to_string(&request).unwrap();
        let parsed: TdfShareRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, request);
    }

    #[test]
    fn tdf_share_response_serialization() {
        let response = TdfShareResponse {
            accepted: true,
            offer_id: "offer-123".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: TdfShareResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, response);
    }

    #[test]
    fn tdf_offers_response_serialization() {
        let response = TdfOffersResponse {
            offers: vec![TdfOffer {
                offer_id: "offer-123".to_string(),
                ticket: "blobaaaa...".to_string(),
                content_hash: "abc123".to_string(),
                size_bytes: 2048,
                policy_attributes: vec!["https://arkavo.net/attr/role".to_string()],
                kas_url: "https://kas.arkavo.net".to_string(),
                sender_agent_id: "agent-002".to_string(),
                received_at: "2026-03-16T12:00:00Z".to_string(),
            }],
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: TdfOffersResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, response);
        assert_eq!(parsed.offers.len(), 1);
    }

    #[test]
    fn tdf_offers_request_default() {
        let request = TdfOffersRequest::default();
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(json, "{}");
    }
}
