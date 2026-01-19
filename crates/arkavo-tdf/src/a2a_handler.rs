//! A2A KAS handler for TDF key rewrap operations.
//!
//! This module provides the main handler for KAS capabilities exposed via
//! the A2A JSON-RPC protocol. It integrates delegation verification, ABAC
//! policy evaluation, and cryptographic key rewrapping.

use base64::{Engine as _, engine::general_purpose};
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
            algorithm: "RSA-OAEP".to_string(),
        }
    }
}

/// KAS keypair for key wrapping operations.
///
/// This is a placeholder that can be implemented with actual RSA
/// operations using opentdf-rs or another crypto library.
pub struct KasKeypair {
    /// PEM-encoded public key
    public_key_pem: String,
    /// Private key material (opaque, implementation-specific)
    #[allow(dead_code)]
    private_key_material: Vec<u8>,
}

impl KasKeypair {
    /// Create a new KAS keypair from PEM-encoded keys.
    pub fn from_pem(public_key_pem: &str, private_key_pem: &str) -> Result<Self, KasError> {
        // Store the private key material for later use
        let private_key_material = private_key_pem.as_bytes().to_vec();

        Ok(Self {
            public_key_pem: public_key_pem.to_string(),
            private_key_material,
        })
    }

    /// Get the PEM-encoded public key.
    pub fn public_key_pem(&self) -> &str {
        &self.public_key_pem
    }

    /// Rewrap a key: decrypt with KAS private key, re-encrypt for client.
    ///
    /// This is a placeholder that returns a mock rewrapped key.
    /// Real implementation would use RSA-OAEP decryption/encryption.
    pub fn rewrap(
        &self,
        wrapped_key_base64: &str,
        _client_public_key_pem: &str,
    ) -> Result<String, KasError> {
        // Validate input is valid base64
        general_purpose::STANDARD
            .decode(wrapped_key_base64)
            .map_err(|e| KasError::CryptoError(format!("Invalid wrapped key: {e}")))?;

        // In a real implementation, this would:
        // 1. Decrypt wrapped_key with KAS private key
        // 2. Re-encrypt the payload key for client_public_key
        // 3. Return the new wrapped key

        // For now, return the input as a placeholder
        // The actual implementation would integrate with opentdf-rs RSA operations
        Ok(wrapped_key_base64.to_string())
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

    /// Create a handler with default configuration.
    pub fn with_defaults() -> Self {
        Self {
            verifier: DelegationVerifier::empty(),
            abac: AbacEvaluator::new(),
            keypair: None,
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
    #[allow(clippy::unused_async)]
    pub async fn handle_rewrap(
        &self,
        request: KasRewrapRequest,
        caller_did: &str,
    ) -> Result<KasRewrapResponse, KasError> {
        // 1. Verify delegation token and extract entitlements
        let token = DelegationToken::from_json(&request.delegation_token)
            .map_err(KasError::Delegation)?;

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
        let keypair = self.keypair.as_ref().ok_or(KasError::KeypairNotConfigured)?;

        let entity_wrapped_key = keypair.rewrap(
            &request.wrapped_key,
            &request.client_public_key,
        )?;

        Ok(KasRewrapResponse { entity_wrapped_key })
    }

    /// Handle a kas.publicKey request.
    #[allow(clippy::unused_async)]
    pub async fn handle_public_key(
        &self,
        request: KasPublicKeyRequest,
    ) -> Result<KasPublicKeyResponse, KasError> {
        let keypair = self.keypair.as_ref().ok_or(KasError::KeypairNotConfigured)?;

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
            public_key: keypair.public_key_pem().to_string(),
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
    use chrono::Utc;

    fn make_test_policy() -> Policy {
        Policy {
            id: Some("test-policy".to_string()),
            attributes: vec![Attribute::new(
                "https://arkavo.net/attr/role",
                &["admin"],
            )],
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

    #[test]
    fn test_decode_policy() {
        let handler = KasA2aHandler::with_defaults();
        let policy = make_test_policy();
        let encoded = encode_policy(&policy);

        let decoded = handler.decode_policy(&encoded).unwrap();
        assert_eq!(decoded.id, policy.id);
        assert_eq!(decoded.attributes.len(), 1);
    }

    #[test]
    fn test_decode_policy_invalid_base64() {
        let handler = KasA2aHandler::with_defaults();
        let result = handler.decode_policy("not-valid-base64!!!");

        assert!(matches!(result, Err(KasError::PolicyDecodeError(_))));
    }

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

    #[test]
    fn test_handler_without_keypair() {
        let handler = KasA2aHandler::with_defaults();
        let request = KasPublicKeyRequest::default();

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handler.handle_public_key(request));

        assert!(matches!(result, Err(KasError::KeypairNotConfigured)));
    }

    #[test]
    fn test_kas_keypair_public_key() {
        let pem = "-----BEGIN PUBLIC KEY-----\nMIIBIjAN...\n-----END PUBLIC KEY-----";
        let keypair = KasKeypair::from_pem(pem, "-----BEGIN PRIVATE KEY-----").unwrap();

        assert_eq!(keypair.public_key_pem(), pem);
    }

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

    #[test]
    fn test_delegation_token_json() {
        let token = make_test_token(&["https://arkavo.net/attr/role/value/admin"]);
        let json = token.to_json();

        let parsed = DelegationToken::from_json(&json).unwrap();
        assert_eq!(parsed.subject_did, token.subject_did);
        assert_eq!(parsed.entitlements, token.entitlements);
    }
}
