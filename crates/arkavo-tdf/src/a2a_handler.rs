//! A2A KAS handler for TDF key rewrap operations.
//!
//! This module provides the main handler for KAS capabilities exposed via
//! the A2A JSON-RPC protocol. It integrates delegation verification, ABAC
//! policy evaluation, and cryptographic key rewrapping.

use arkavo_crypto::{KasEcKeypair, KasEcPublicKey};
use base64::{Engine as _, engine::general_purpose};
use hkdf::Hkdf;
use opentdf_kas::{
    KasEcKeypair as KasServerKeypair, NanoTdfVersion, compute_nanotdf_salt, custom_ecdh,
    detect_nanotdf_version, ec_unwrap, p256,
};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

mod policy_binding;

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
        let header_bytes = decode_header(wrapped_key_base64)?;

        // Parse client's EC public key for session ECDH
        let client_public = KasEcPublicKey::from_base64(client_public_key_base64)
            .map_err(|e| KasError::InvalidKeyFormat(format!("Invalid client public key: {e}")))?;

        // Compute session shared secret with client
        let session_shared_secret = Zeroizing::new(self.keypair.diffie_hellman(&client_public));

        // Use kas_server for full rewrap with HKDF + AES-GCM
        let rewrapped = ec_unwrap(
            &header_bytes,
            ephemeral_key(&header_bytes),
            self.server_keypair.private_key(),
            &session_shared_secret,
        )
        .map_err(|e| KasError::CryptoError(e.to_string()))?;

        Ok(rewrapped)
    }

    /// Recover the DEK a wrapped key carries.
    ///
    /// `ec_unwrap` derives the DEK internally and never returns it, so this
    /// repeats its derivation: ECDH with the header's ephemeral key, then
    /// HKDF-SHA256 salted by the header's NanoTDF version. The policy binding
    /// is checked against this key, so it must stay the same derivation.
    fn unwrap_dek(&self, wrapped_key_base64: &str) -> Result<Zeroizing<[u8; 32]>, KasError> {
        let header_bytes = decode_header(wrapped_key_base64)?;
        let ephemeral = p256::PublicKey::from_sec1_bytes(ephemeral_key(&header_bytes))
            .map_err(|e| KasError::CryptoError(format!("Invalid ephemeral key: {e}")))?;
        let shared = Zeroizing::new(
            custom_ecdh(self.server_keypair.private_key(), &ephemeral)
                .map_err(|e| KasError::CryptoError(e.to_string()))?,
        );
        let salt = compute_nanotdf_salt(
            detect_nanotdf_version(&header_bytes).unwrap_or(NanoTdfVersion::V12),
        );

        let mut dek = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(Some(&salt), &shared)
            .expand(b"", dek.as_mut())
            .map_err(|e| KasError::CryptoError(format!("DEK derivation: {e}")))?;
        Ok(dek)
    }
}

/// Bytes this KAS reads from a wrapped-key header: the 3-byte NanoTDF magic
/// and version, then the 33-byte compressed P-256 ephemeral key.
const HEADER_LEN: usize = 36;

/// Decode a base64 wrapped-key header, refusing one too short to hold the
/// ephemeral key.
fn decode_header(wrapped_key_base64: &str) -> Result<Vec<u8>, KasError> {
    let header_bytes = general_purpose::STANDARD
        .decode(wrapped_key_base64)
        .map_err(|e| KasError::CryptoError(format!("Invalid wrapped key: {e}")))?;
    if header_bytes.len() < HEADER_LEN {
        return Err(KasError::CryptoError(
            "Header too short for NanoTDF".to_string(),
        ));
    }
    Ok(header_bytes)
}

/// The compressed ephemeral key in a header `decode_header` accepted.
fn ephemeral_key(header_bytes: &[u8]) -> &[u8] {
    &header_bytes[3..HEADER_LEN]
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
    /// 4. Verify the policy binding against the unwrapped DEK, so the policy
    ///    the decision uses is the one the data was sealed with
    /// 5. Evaluate ABAC policy against entitlements
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

        // 3. Verify the policy binding before the policy decides anything
        let keypair = self
            .keypair
            .as_ref()
            .ok_or(KasError::KeypairNotConfigured)?;
        let dek = keypair.unwrap_dek(&request.wrapped_key)?;
        policy_binding::verify(&request.policy_binding, &request.policy, dek.as_ref())?;

        // 4. Evaluate ABAC policy
        let decision = self
            .abac
            .evaluate(&entitlements, &policy)
            .map_err(|e| KasError::Abac(e.to_string()))?;

        if decision != Decision::Permit {
            return Err(KasError::AccessDenied);
        }

        // 5. Rewrap the key for the client
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
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
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

    const ROLE: &str = "https://arkavo.net/attr/role";
    const ADMIN: &str = "https://arkavo.net/attr/role/value/admin";

    /// A KAS that trusts one root, and a caller that root delegated
    /// `entitlements` to.
    struct Fixture {
        handler: KasA2aHandler,
        kas_public: KasEcPublicKey,
        caller_did: String,
        token_json: String,
    }

    fn fixture(entitlements: &[&str]) -> Fixture {
        let root = arkavo_crypto::AgentKeypair::generate();
        let root_did = root.public_key().to_did_key();
        let caller_did = arkavo_crypto::AgentKeypair::generate()
            .public_key()
            .to_did_key();
        let mut token = DelegationToken {
            issuer_did: root_did.clone(),
            subject_did: caller_did.clone(),
            entitlements: entitlements.iter().map(|e| (*e).to_string()).collect(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            signature: String::new(),
            parent: None,
        };
        token.signature =
            general_purpose::STANDARD.encode(root.sign(&token.payload_bytes().unwrap()));

        let keypair = KasKeypair::generate();
        let kas_public = KasEcPublicKey::from_base64(&keypair.public_key_base64()).unwrap();
        let mut handler = KasA2aHandler::new(
            vec![TrustedRoot {
                did: root_did,
                public_key_bytes: root.public_key().to_bytes(),
            }],
            KasA2aConfig::default(),
        );
        handler.set_keypair(keypair);

        Fixture {
            handler,
            kas_public,
            caller_did,
            token_json: token.to_json().unwrap(),
        }
    }

    /// Wrap a fresh DEK for the KAS as a NanoTDF v1.2 writer does, from the
    /// writer's side of the ECDH, so it does not share code with `unwrap_dek`.
    fn seal(kas_public: &KasEcPublicKey) -> (String, [u8; 32]) {
        let ephemeral = KasEcKeypair::generate();
        let mut header = b"L1L".to_vec();
        header.extend_from_slice(&ephemeral.public_key_sec1_compressed());
        let shared = ephemeral.diffie_hellman(kas_public);
        let mut dek = [0u8; 32];
        Hkdf::<Sha256>::new(Some(&compute_nanotdf_salt(NanoTdfVersion::V12)), &shared)
            .expand(b"", &mut dek)
            .unwrap();
        (general_purpose::STANDARD.encode(header), dek)
    }

    fn mac(dek: &[u8], policy_base64: &str) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<Sha256>::new_from_slice(dek).unwrap();
        mac.update(policy_base64.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    fn spec_binding(dek: &[u8], policy_base64: &str) -> PolicyBinding {
        PolicyBinding::new(&general_purpose::STANDARD.encode(mac(dek, policy_base64)))
    }

    fn legacy_binding(dek: &[u8], policy_base64: &str) -> PolicyBinding {
        PolicyBinding::new(&general_purpose::STANDARD.encode(hex::encode(mac(dek, policy_base64))))
    }

    fn policy_requiring(role: &str) -> String {
        encode_policy(&Policy {
            id: Some(format!("needs-{role}")),
            attributes: vec![Attribute::new(ROLE, &[role])],
            dissemination: vec![],
        })
    }

    fn rewrap_request(
        f: &Fixture,
        wrapped_key: String,
        policy: String,
        policy_binding: PolicyBinding,
        client: &KasEcKeypair,
    ) -> KasRewrapRequest {
        KasRewrapRequest {
            wrapped_key,
            policy_binding,
            policy,
            delegation_token: f.token_json.clone(),
            client_public_key: client.public_key_base64(),
        }
    }

    /// Open the rewrapped DEK as the client does: ECDH with the KAS key, HKDF
    /// with the v1.2 salt, then AES-256-GCM over nonce || ciphertext || tag.
    fn open(
        entity_wrapped_key: &str,
        client: &KasEcKeypair,
        kas_public: &KasEcPublicKey,
    ) -> Vec<u8> {
        use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce, aead::Aead};
        let session = client.diffie_hellman(kas_public);
        let mut key = Key::<Aes256Gcm>::default();
        Hkdf::<Sha256>::new(Some(&compute_nanotdf_salt(NanoTdfVersion::V12)), &session)
            .expand(b"", &mut key)
            .unwrap();
        let bytes = general_purpose::STANDARD
            .decode(entity_wrapped_key)
            .unwrap();
        let (nonce, ciphertext) = bytes.split_at(12);
        Aes256Gcm::new(&key)
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .unwrap()
    }

    #[spec("TDFS-006")]
    #[spec("TDF-008")]
    #[tokio::test]
    async fn rewrap_releases_the_dek_its_binding_was_checked_against() {
        let f = fixture(&[ADMIN]);
        let (wrapped_key, dek) = seal(&f.kas_public);
        let policy = policy_requiring("admin");
        let binding = spec_binding(&dek, &policy);
        let client = KasEcKeypair::generate();

        let response = f
            .handler
            .handle_rewrap(
                rewrap_request(&f, wrapped_key, policy, binding, &client),
                &f.caller_did,
            )
            .await
            .unwrap();

        assert_eq!(
            open(&response.entity_wrapped_key, &client, &f.kas_public),
            dek
        );
    }

    #[spec("TDFS-006")]
    #[tokio::test]
    async fn rewrap_accepts_the_legacy_hex_binding() {
        let f = fixture(&[ADMIN]);
        let (wrapped_key, dek) = seal(&f.kas_public);
        let policy = policy_requiring("admin");
        let binding = legacy_binding(&dek, &policy);
        let client = KasEcKeypair::generate();

        let result = f
            .handler
            .handle_rewrap(
                rewrap_request(&f, wrapped_key, policy, binding, &client),
                &f.caller_did,
            )
            .await;

        assert!(result.is_ok(), "{result:?}");
    }

    #[spec("TDFS-006")]
    #[tokio::test]
    async fn rewrap_refuses_a_policy_swapped_for_one_the_caller_satisfies() {
        // The data was sealed for "secret". The caller holds only "admin", so
        // it sends an "admin" policy with the original binding.
        let f = fixture(&[ADMIN]);
        let (wrapped_key, dek) = seal(&f.kas_public);
        let binding = spec_binding(&dek, &policy_requiring("secret"));
        let client = KasEcKeypair::generate();

        let result = f
            .handler
            .handle_rewrap(
                rewrap_request(&f, wrapped_key, policy_requiring("admin"), binding, &client),
                &f.caller_did,
            )
            .await;

        assert!(
            matches!(result, Err(KasError::PolicyBindingInvalid(_))),
            "{result:?}"
        );
    }

    #[spec("TDFS-006")]
    #[tokio::test]
    async fn rewrap_refuses_a_binding_made_with_another_dek() {
        let f = fixture(&[ADMIN]);
        let (wrapped_key, _) = seal(&f.kas_public);
        let (_, other_dek) = seal(&f.kas_public);
        let policy = policy_requiring("admin");
        let binding = spec_binding(&other_dek, &policy);
        let client = KasEcKeypair::generate();

        let result = f
            .handler
            .handle_rewrap(
                rewrap_request(&f, wrapped_key, policy, binding, &client),
                &f.caller_did,
            )
            .await;

        assert!(
            matches!(result, Err(KasError::PolicyBindingInvalid(_))),
            "{result:?}"
        );
    }

    #[spec("TDFS-006")]
    #[tokio::test]
    async fn rewrap_refuses_a_placeholder_binding() {
        // Regression: the handler used to accept any non-empty HS256 hash.
        let f = fixture(&[ADMIN]);
        let (wrapped_key, _) = seal(&f.kas_public);
        let client = KasEcKeypair::generate();

        for binding in [PolicyBinding::new("test-hash"), PolicyBinding::default()] {
            let result = f
                .handler
                .handle_rewrap(
                    rewrap_request(
                        &f,
                        wrapped_key.clone(),
                        policy_requiring("admin"),
                        binding,
                        &client,
                    ),
                    &f.caller_did,
                )
                .await;
            assert!(
                matches!(result, Err(KasError::PolicyBindingInvalid(_))),
                "{result:?}"
            );
        }
    }

    #[spec("TDF-008")]
    #[tokio::test]
    async fn rewrap_with_a_valid_binding_still_needs_the_entitlement() {
        let f = fixture(&["https://arkavo.net/attr/role/value/viewer"]);
        let (wrapped_key, dek) = seal(&f.kas_public);
        let policy = policy_requiring("admin");
        let binding = spec_binding(&dek, &policy);
        let client = KasEcKeypair::generate();

        let result = f
            .handler
            .handle_rewrap(
                rewrap_request(&f, wrapped_key, policy, binding, &client),
                &f.caller_did,
            )
            .await;

        assert!(matches!(result, Err(KasError::AccessDenied)), "{result:?}");
    }

    #[spec("TDFS-011")]
    #[tokio::test]
    async fn test_handler_without_keypair() {
        // Use new() without a keypair to test the error case
        let handler = KasA2aHandler::new(vec![], KasA2aConfig::default());
        let request = KasPublicKeyRequest::default();

        let result = handler.handle_public_key(request).await;

        assert!(matches!(result, Err(KasError::KeypairNotConfigured)));
    }

    #[spec("TDFS-011")]
    #[tokio::test]
    async fn test_handler_with_defaults_has_keypair() {
        // with_defaults() should generate a keypair
        let handler = KasA2aHandler::with_defaults();
        let request = KasPublicKeyRequest::default();

        let result = handler.handle_public_key(request).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.public_key.is_empty());
        assert_eq!(response.algorithm, "ec:secp256r1");
    }

    #[spec("TDFS-011")]
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
