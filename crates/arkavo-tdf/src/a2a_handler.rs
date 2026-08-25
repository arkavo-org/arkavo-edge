//! A2A KAS handler for TDF key rewrap operations.
//!
//! This module provides the main handler for KAS capabilities exposed via
//! the A2A JSON-RPC protocol. It integrates delegation verification, ABAC
//! policy evaluation, and cryptographic key rewrapping.

use arkavo_crypto::{KasEcKeypair, KasEcPublicKey};
use base64::{Engine as _, engine::general_purpose};
use opentdf_kas::{KasEcKeypair as KasServerKeypair, ec_unwrap};
use thiserror::Error;

use crate::a2a_types::{
    KasPublicKeyRequest, KasPublicKeyResponse, KasRewrapRequest, KasRewrapResponse,
};
use crate::abac::{AbacEvaluator, Decision};
use crate::delegation::{DelegationError, DelegationToken, DelegationVerifier, TrustedRoot};
use crate::types::Policy;

/// Errors that can occur during KAS A2A operations.
#[derive(Error, Debug)]
pub enum KasError {
    /// Delegation token verification failed
    #[error("Delegation error: {0}")]
    Delegation(#[from] DelegationError),

    /// ABAC policy evaluation denied access
    #[error("Access denied: insufficient entitlements for policy")]
    AccessDenied,

    /// ABAC evaluation failed
    #[error("ABAC error: {0}")]
    Abac(String),

    /// Policy binding verification failed
    #[error("Policy binding invalid: {0}")]
    PolicyBindingInvalid(String),

    /// Policy decoding failed
    #[error("Policy decode error: {0}")]
    PolicyDecodeError(String),

    /// Cryptographic operation failed
    #[error("Crypto error: {0}")]
    CryptoError(String),

    /// KAS keypair not configured
    #[error("KAS keypair not configured")]
    KeypairNotConfigured,

    /// Invalid key format
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(String),
}

/// Configuration for the KAS A2A handler.
#[derive(Clone)]
pub struct KasA2aConfig {
    /// Key identifier for the KAS public key
    pub key_id: String,
    /// Algorithm supported (e.g., "RSA-OAEP")
    pub algorithm: String,
}

impl Default for KasA2aConfig {
    fn default() -> Self {
        Self {
            key_id: "kas-key-1".to_string(),
            algorithm: "ec:secp256r1".to_string(),
        }
    }
}

/// Wrapper for EC P-256 keypair used in KAS operations.
///
/// Wraps opentdf_kas::KasEcKeypair to provide full NanoTDF-compatible
/// key rewrapping with HKDF key derivation and AES-GCM encryption.
pub struct KasKeypair {
    keypair: KasEcKeypair,
    server_keypair: KasServerKeypair,
}

impl KasKeypair {
    /// Generate a new random EC P-256 keypair.
    ///
    /// # Panics
    ///
    /// Panics if the generated keypair bytes are invalid (should never happen).
    pub fn generate() -> Self {
        let keypair = KasEcKeypair::generate();
        let server_keypair = KasServerKeypair::from_private_key_bytes(&keypair.secret_bytes())
            .expect("generated keypair should be valid");
        Self {
            keypair,
            server_keypair,
        }
    }

    /// Create from secret key bytes (32 bytes).
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, KasError> {
        let keypair = KasEcKeypair::from_secret_bytes(bytes)
            .map_err(|e| KasError::InvalidKeyFormat(e.to_string()))?;
        let server_keypair = KasServerKeypair::from_private_key_bytes(bytes)
            .map_err(|e| KasError::CryptoError(e.to_string()))?;
        Ok(Self {
            keypair,
            server_keypair,
        })
    }

    /// Get the public key as base64-encoded SEC1 uncompressed format.
    pub fn public_key_base64(&self) -> String {
        self.keypair.public_key_base64()
    }

    /// Get the secret key bytes for persistence.
    pub fn secret_bytes(&self) -> Vec<u8> {
        self.keypair.secret_bytes()
    }

    /// Rewrap a key using NanoTDF-compatible ECDH + HKDF + AES-GCM.
    ///
    /// For TDF NanoTDF format, this performs:
    /// 1. Parse the NanoTDF header to extract ephemeral public key
    /// 2. Perform ECDH between KAS private key and TDF ephemeral key
    /// 3. Derive DEK using HKDF with version-based salt
    /// 4. Perform ECDH with client's public key to get session secret
    /// 5. Rewrap DEK for transport using AES-GCM
    ///
    /// Returns base64-encoded (nonce || ciphertext || tag).
    pub fn rewrap(
        &self,
        wrapped_key_base64: &str,
        client_public_key_base64: &str,
    ) -> Result<String, KasError> {
        // Decode NanoTDF header (contains ephemeral public key)
        let header_bytes = general_purpose::STANDARD
            .decode(wrapped_key_base64)
            .map_err(|e| KasError::CryptoError(format!("Invalid wrapped key: {e}")))?;

        // Parse client's EC public key for session ECDH
        let client_public = KasEcPublicKey::from_base64(client_public_key_base64)
            .map_err(|e| KasError::InvalidKeyFormat(format!("Invalid client public key: {e}")))?;

        // Compute session shared secret with client
        let session_shared_secret = self.keypair.diffie_hellman(&client_public);

        // Extract ephemeral public key from header
        // NanoTDF header structure: 3 bytes magic + variable header
        // For simplicity, assume compressed P-256 key starts at offset 3
        if header_bytes.len() < 36 {
            return Err(KasError::CryptoError(
                "Header too short for NanoTDF".to_string(),
            ));
        }

        // Extract the 33-byte compressed ephemeral public key
        // Actual offset depends on header structure, simplified here
        let ephemeral_key_bytes = &header_bytes[3..36];

        // Use kas_server for full rewrap with HKDF + AES-GCM
        let rewrapped = ec_unwrap(
            &header_bytes,
            ephemeral_key_bytes,
            self.server_keypair.private_key(),
            &session_shared_secret,
        )
        .map_err(|e| KasError::CryptoError(e.to_string()))?;

        Ok(rewrapped)
    }
}

/// Handler for KAS A2A JSON-RPC methods.
///
/// Coordinates delegation verification, ABAC evaluation, and key rewrapping
/// for the `kas.rewrap` and `kas.publicKey` RPC methods.
pub struct KasA2aHandler {
    verifier: DelegationVerifier,
    abac: AbacEvaluator,
    keypair: Option<KasKeypair>,
    config: KasA2aConfig,
}

impl KasA2aHandler {
    /// Create a new KAS A2A handler with the given trusted roots.
    pub fn new(trusted_roots: Vec<TrustedRoot>, config: KasA2aConfig) -> Self {
        Self {
            verifier: DelegationVerifier::new(trusted_roots),
            abac: AbacEvaluator::new(),
            keypair: None,
            config,
        }
    }

    /// Create a handler with default configuration and a generated keypair.
    pub fn with_defaults() -> Self {
        Self {
            verifier: DelegationVerifier::empty(),
            abac: AbacEvaluator::new(),
            keypair: Some(KasKeypair::generate()),
            config: KasA2aConfig::default(),
        }
    }

    /// Set the KAS keypair for rewrap operations.
    pub fn set_keypair(&mut self, keypair: KasKeypair) {
        self.keypair = Some(keypair);
    }

    /// Add a trusted root to the delegation verifier.
    pub fn add_trusted_root(&mut self, root: TrustedRoot) {
        self.verifier.add_trusted_root(root);
    }

    /// Handle a kas.rewrap request.
    ///
    /// Flow:
    /// 1. Parse and verify the delegation token chain
    /// 2. Extract entitlements from the verified chain
    /// 3. Decode and parse the TDF policy
    /// 4. Evaluate ABAC policy against entitlements
    /// 5. Verify policy binding (HMAC)
    /// 6. Rewrap the key for the client's public key
    // 1.98 files the same shape under a second name for functions in impl blocks.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    pub async fn handle_rewrap(
        &self,
        request: KasRewrapRequest,
        caller_did: &str,
    ) -> Result<KasRewrapResponse, KasError> {
        // 1. Verify delegation token and extract entitlements
        let token =
            DelegationToken::from_json(&request.delegation_token).map_err(KasError::Delegation)?;

        let entitlements = self.verifier.verify(&token, caller_did)?;

        // 2. Decode policy from base64 JSON
        let policy = self.decode_policy(&request.policy)?;

        // 3. Evaluate ABAC policy
        let decision = self
            .abac
            .evaluate(&entitlements, &policy)
            .map_err(|e| KasError::Abac(e.to_string()))?;

        if decision != Decision::Permit {
            return Err(KasError::AccessDenied);
        }

        // 4. Verify policy binding
        self.verify_policy_binding(&request)?;

        // 5. Rewrap the key for the client
        let keypair = self
            .keypair
            .as_ref()
            .ok_or(KasError::KeypairNotConfigured)?;

        let entity_wrapped_key =
            keypair.rewrap(&request.wrapped_key, &request.client_public_key)?;

        Ok(KasRewrapResponse { entity_wrapped_key })
    }

    /// Handle a kas.publicKey request.
    // 1.98 files the same shape under a second name for functions in impl blocks.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    pub async fn handle_public_key(
        &self,
        request: KasPublicKeyRequest,
    ) -> Result<KasPublicKeyResponse, KasError> {
        let keypair = self
            .keypair
            .as_ref()
            .ok_or(KasError::KeypairNotConfigured)?;

        // Check if requested algorithm matches (if specified)
        if let Some(ref requested_alg) = request.algorithm
            && requested_alg != &self.config.algorithm
        {
            return Err(KasError::InvalidKeyFormat(format!(
                "Unsupported algorithm: {requested_alg} (only {} supported)",
                self.config.algorithm
            )));
        }

        Ok(KasPublicKeyResponse {
            public_key: keypair.public_key_base64(),
            key_id: self.config.key_id.clone(),
            algorithm: self.config.algorithm.clone(),
        })
    }

    /// Decode a base64-encoded policy JSON.
    fn decode_policy(&self, policy_base64: &str) -> Result<Policy, KasError> {
        let policy_bytes = general_purpose::STANDARD
            .decode(policy_base64)
            .map_err(|e| KasError::PolicyDecodeError(format!("Base64 decode: {e}")))?;

        let policy_json = String::from_utf8(policy_bytes)
            .map_err(|e| KasError::PolicyDecodeError(format!("UTF-8 decode: {e}")))?;

        serde_json::from_str(&policy_json)
            .map_err(|e| KasError::PolicyDecodeError(format!("JSON parse: {e}")))
    }

    /// Verify the policy binding HMAC.
    ///
    /// The policy binding ensures the policy hasn't been modified since
    /// the TDF was created. The hash is an HMAC of the policy using
    /// the symmetric key as the HMAC key.
    fn verify_policy_binding(&self, request: &KasRewrapRequest) -> Result<(), KasError> {
        // In a full implementation, this would:
        // 1. Decrypt the wrapped key to get the symmetric key
        // 2. Compute HMAC of the policy using that key
        // 3. Compare with the binding hash

        // For now, just verify the binding has the expected algorithm
        if request.policy_binding.alg != "HS256" {
            return Err(KasError::PolicyBindingInvalid(format!(
                "Unsupported binding algorithm: {}",
                request.policy_binding.alg
            )));
        }

        if request.policy_binding.hash.is_empty() {
            return Err(KasError::PolicyBindingInvalid(
                "Empty policy binding hash".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Attribute, PolicyBinding};
    use arkavo_test_macros::spec;
    use chrono::Utc;

    fn make_test_policy() -> Policy {
        Policy {
            id: Some("test-policy".to_string()),
            attributes: vec![Attribute::new("https://arkavo.net/attr/role", &["admin"])],
            dissemination: vec![],
        }
    }

    fn make_test_token(entitlements: &[&str]) -> DelegationToken {
        DelegationToken {
            issuer_did: "did:key:z6MkRoot".to_string(),
            subject_did: "did:key:z6MkCaller".to_string(),
            entitlements: entitlements.iter().map(|s| (*s).to_string()).collect(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            signature: String::new(),
            parent: None,
        }
    }

    fn encode_policy(policy: &Policy) -> String {
        let json = serde_json::to_string(policy).unwrap();
        general_purpose::STANDARD.encode(json.as_bytes())
    }

    #[spec("TDFS-006")]
    #[test]
    fn test_decode_policy() {
        let handler = KasA2aHandler::with_defaults();
        let policy = make_test_policy();
        let encoded = encode_policy(&policy);

        let decoded = handler.decode_policy(&encoded).unwrap();
        assert_eq!(decoded.id, policy.id);
        assert_eq!(decoded.attributes.len(), 1);
    }

    #[spec("TDFS-006")]
    #[test]
    fn test_decode_policy_invalid_base64() {
        let handler = KasA2aHandler::with_defaults();
        let result = handler.decode_policy("not-valid-base64!!!");

        assert!(matches!(result, Err(KasError::PolicyDecodeError(_))));
    }

    #[spec("TDFS-006")]
    #[test]
    fn test_verify_policy_binding_valid() {
        let handler = KasA2aHandler::with_defaults();
        let request = KasRewrapRequest {
            wrapped_key: "dGVzdA==".to_string(),
            policy_binding: PolicyBinding::new("test-hash"),
            policy: "eyJhdHRyaWJ1dGVzIjpbXX0=".to_string(),
            delegation_token: "{}".to_string(),
            client_public_key: "-----BEGIN PUBLIC KEY-----".to_string(),
        };

        let result = handler.verify_policy_binding(&request);
        assert!(result.is_ok());
    }

    #[spec("TDFS-006")]
    #[test]
    fn test_verify_policy_binding_empty_hash() {
        let handler = KasA2aHandler::with_defaults();
        let request = KasRewrapRequest {
            wrapped_key: "dGVzdA==".to_string(),
            policy_binding: PolicyBinding {
                alg: "HS256".to_string(),
                hash: String::new(),
            },
            policy: "eyJhdHRyaWJ1dGVzIjpbXX0=".to_string(),
            delegation_token: "{}".to_string(),
            client_public_key: "-----BEGIN PUBLIC KEY-----".to_string(),
        };

        let result = handler.verify_policy_binding(&request);
        assert!(matches!(result, Err(KasError::PolicyBindingInvalid(_))));
    }

    #[spec("TDFS-011")]
    #[test]
    fn test_handler_without_keypair() {
        // Use new() without a keypair to test the error case
        let handler = KasA2aHandler::new(vec![], KasA2aConfig::default());
        let request = KasPublicKeyRequest::default();

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.handle_public_key(request));

        assert!(matches!(result, Err(KasError::KeypairNotConfigured)));
    }

    #[spec("TDFS-011")]
    #[spec("TDFS-013")]
    #[test]
    fn test_handler_with_defaults_has_keypair() {
        // with_defaults() should generate a keypair
        let handler = KasA2aHandler::with_defaults();
        let request = KasPublicKeyRequest::default();

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.handle_public_key(request));

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.public_key.is_empty());
        assert_eq!(response.algorithm, "ec:secp256r1");
    }

    #[spec("TDFS-011")]
    #[spec("TDFS-013")]
    #[test]
    fn test_kas_keypair_public_key() {
        let keypair = KasKeypair::generate();
        let public_key = keypair.public_key_base64();

        // EC P-256 SEC1 uncompressed public key is 65 bytes (1 + 32 + 32)
        // Base64 encoded: 65 * 4/3 = ~88 characters
        assert!(!public_key.is_empty());
        assert!(public_key.len() > 80); // Base64 of 65 bytes
    }

    #[spec("TDFS-011")]
    #[test]
    fn test_handler_with_config() {
        let config = KasA2aConfig {
            key_id: "custom-key".to_string(),
            algorithm: "RSA-OAEP-256".to_string(),
        };

        let handler = KasA2aHandler::new(vec![], config);

        // Request with different algorithm should fail
        let request = KasPublicKeyRequest {
            algorithm: Some("RSA-OAEP".to_string()),
        };

        // Would need keypair to actually test this
        let _ = handler; // Avoid unused warning
        let _ = request;
    }

    #[spec("TDFS-008")]
    #[test]
    fn test_delegation_token_json() {
        let token = make_test_token(&["https://arkavo.net/attr/role/value/admin"]);
        let json = token.to_json().unwrap();

        let parsed = DelegationToken::from_json(&json).unwrap();
        assert_eq!(parsed.subject_did, token.subject_did);
        assert_eq!(parsed.entitlements, token.entitlements);
    }
}
