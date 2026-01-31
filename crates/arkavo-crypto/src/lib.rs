use base64::{Engine as _, engine::general_purpose};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey as P256PublicKey, SecretKey as P256SecretKey};
use std::fmt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Invalid signature")]
    InvalidSignature,
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(String),
    #[error("Signature verification failed")]
    VerificationFailed,
}

pub struct AgentKeypair {
    signing_key: SigningKey,
}

impl AgentKeypair {
    pub fn generate() -> Self {
        // SECURITY FIX (HIGH-001): Use cryptographically secure RNG
        use rand::RngCore;
        use rand::rngs::OsRng;

        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        let signing_key = SigningKey::from_bytes(&key_bytes);
        Self { signing_key }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidKeyFormat(format!(
                "Expected 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(bytes);
        let signing_key = SigningKey::from_bytes(&key_bytes);
        Ok(Self { signing_key })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.signing_key.to_bytes().to_vec()
    }

    pub fn public_key(&self) -> AgentPublicKey {
        AgentPublicKey {
            verifying_key: self.signing_key.verifying_key(),
        }
    }

    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.signing_key.sign(message).to_bytes().to_vec()
    }
}

#[derive(Clone)]
pub struct AgentPublicKey {
    verifying_key: VerifyingKey,
}

impl AgentPublicKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() != 32 {
            return Err(CryptoError::InvalidKeyFormat(format!(
                "Expected 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(bytes);
        let verifying_key = VerifyingKey::from_bytes(&key_bytes)
            .map_err(|e| CryptoError::InvalidKeyFormat(e.to_string()))?;
        Ok(Self { verifying_key })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.verifying_key.to_bytes().to_vec()
    }

    pub fn to_base64(&self) -> String {
        general_purpose::STANDARD.encode(self.to_bytes())
    }

    pub fn from_base64(s: &str) -> Result<Self, CryptoError> {
        let bytes = general_purpose::STANDARD
            .decode(s)
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Base64 decode error: {}", e)))?;
        Self::from_bytes(&bytes)
    }

    /// Convert to DID:key format for Ed25519 public keys.
    ///
    /// Format: `did:key:z{base58btc(0xed01 || public_key_bytes)}`
    /// - `z` = multibase prefix for base58btc
    /// - `0xed01` = multicodec prefix for Ed25519 public key
    pub fn to_did_key(&self) -> String {
        // Ed25519 public key multicodec prefix
        const ED25519_MULTICODEC: [u8; 2] = [0xed, 0x01];

        let pk_bytes = self.verifying_key.to_bytes();
        let mut prefixed = Vec::with_capacity(2 + pk_bytes.len());
        prefixed.extend_from_slice(&ED25519_MULTICODEC);
        prefixed.extend_from_slice(&pk_bytes);

        // base58btc encode with 'z' multibase prefix
        let encoded = bs58::encode(&prefixed).into_string();
        format!("did:key:z{encoded}")
    }

    /// Parse a DID:key string back to an `AgentPublicKey`.
    ///
    /// Expects format: `did:key:z{base58btc(0xed01 || public_key_bytes)}`
    pub fn from_did_key(did: &str) -> Result<Self, CryptoError> {
        const ED25519_MULTICODEC: [u8; 2] = [0xed, 0x01];
        const EXPECTED_LEN: usize = 34; // 2 bytes prefix + 32 bytes key

        // Validate prefix
        let encoded = did
            .strip_prefix("did:key:z")
            .ok_or_else(|| CryptoError::InvalidKeyFormat("Invalid DID:key prefix".to_string()))?;

        // Decode base58btc
        let decoded = bs58::decode(encoded)
            .into_vec()
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Base58 decode error: {}", e)))?;

        // SECURITY FIX (HIGH-002): Validate exact length
        if decoded.len() != EXPECTED_LEN {
            return Err(CryptoError::InvalidKeyFormat(format!(
                "Invalid DID:key length: expected {}, got {}",
                EXPECTED_LEN,
                decoded.len()
            )));
        }

        // Check multicodec prefix
        if decoded[0] != ED25519_MULTICODEC[0] || decoded[1] != ED25519_MULTICODEC[1] {
            return Err(CryptoError::InvalidKeyFormat(
                "Invalid Ed25519 multicodec prefix".to_string(),
            ));
        }

        // Extract public key bytes (safe due to length check above)
        let pk_bytes = &decoded[2..];
        Self::from_bytes(pk_bytes)
    }

    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), CryptoError> {
        if signature.len() != 64 {
            return Err(CryptoError::InvalidSignature);
        }
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(signature);
        let signature = Signature::from_bytes(&sig_bytes);
        self.verifying_key
            .verify(message, &signature)
            .map_err(|_| CryptoError::VerificationFailed)
    }
}

impl fmt::Display for AgentPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_base64())
    }
}

impl fmt::Debug for AgentPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentPublicKey")
            .field("base64", &self.to_base64())
            .finish()
    }
}

/// EC P-256 keypair for KAS key wrapping operations.
///
/// Used for ECDH-based key agreement in TDF encryption.
pub struct KasEcKeypair {
    secret_key: P256SecretKey,
    public_key: P256PublicKey,
}

impl KasEcKeypair {
    /// Generate a new random EC P-256 keypair.
    pub fn generate() -> Self {
        let secret_key = P256SecretKey::random(&mut rand::thread_rng());
        let public_key = secret_key.public_key();
        Self {
            secret_key,
            public_key,
        }
    }

    /// Create a keypair from raw secret key bytes (32 bytes).
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let secret_key = P256SecretKey::from_slice(bytes)
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Invalid EC secret key: {e}")))?;
        let public_key = secret_key.public_key();
        Ok(Self {
            secret_key,
            public_key,
        })
    }

    /// Get the secret key bytes (32 bytes).
    pub fn secret_bytes(&self) -> Vec<u8> {
        self.secret_key.to_bytes().to_vec()
    }

    /// Get the public key in SEC1 uncompressed format.
    pub fn public_key_sec1(&self) -> Vec<u8> {
        self.public_key.to_encoded_point(false).as_bytes().to_vec()
    }

    /// Get the public key in SEC1 compressed format.
    pub fn public_key_sec1_compressed(&self) -> Vec<u8> {
        self.public_key.to_encoded_point(true).as_bytes().to_vec()
    }

    /// Get the public key as base64-encoded SEC1 uncompressed.
    pub fn public_key_base64(&self) -> String {
        general_purpose::STANDARD.encode(self.public_key_sec1())
    }

    /// Get the public key component.
    pub fn public_key(&self) -> KasEcPublicKey {
        KasEcPublicKey {
            public_key: self.public_key,
        }
    }

    /// Perform ECDH key agreement with a peer's public key.
    ///
    /// Returns the shared secret (32 bytes).
    pub fn diffie_hellman(&self, peer_public: &KasEcPublicKey) -> Vec<u8> {
        use p256::ecdh::diffie_hellman;
        let shared = diffie_hellman(
            self.secret_key.to_nonzero_scalar(),
            peer_public.public_key.as_affine(),
        );
        shared.raw_secret_bytes().to_vec()
    }
}

/// EC P-256 public key for KAS operations.
#[derive(Clone)]
pub struct KasEcPublicKey {
    public_key: P256PublicKey,
}

impl KasEcPublicKey {
    /// Create from SEC1-encoded bytes (compressed or uncompressed).
    pub fn from_sec1_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let public_key = P256PublicKey::from_sec1_bytes(bytes)
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Invalid EC public key: {e}")))?;
        Ok(Self { public_key })
    }

    /// Create from base64-encoded SEC1 bytes.
    pub fn from_base64(s: &str) -> Result<Self, CryptoError> {
        let bytes = general_purpose::STANDARD
            .decode(s)
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Base64 decode error: {e}")))?;
        Self::from_sec1_bytes(&bytes)
    }

    /// Get as SEC1 uncompressed bytes.
    pub fn to_sec1_bytes(&self) -> Vec<u8> {
        self.public_key.to_encoded_point(false).as_bytes().to_vec()
    }

    /// Get as base64-encoded SEC1 uncompressed.
    pub fn to_base64(&self) -> String {
        general_purpose::STANDARD.encode(self.to_sec1_bytes())
    }
}

impl fmt::Display for KasEcPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_base64())
    }
}

impl fmt::Debug for KasEcPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KasEcPublicKey")
            .field("base64", &self.to_base64())
            .finish()
    }
}

/// P-256 ECDSA keypair for iOS Secure Enclave compatibility.
///
/// iOS Secure Enclave uses P-256 (secp256r1) with ECDSA signatures.
/// This type provides signing/verification compatible with iOS.
pub struct P256SigningKeypair {
    signing_key: p256::ecdsa::SigningKey,
}

impl P256SigningKeypair {
    /// Generate a new random P-256 signing keypair.
    pub fn generate() -> Self {
        let signing_key = p256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        Self { signing_key }
    }

    /// Create from raw secret key bytes (32 bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let signing_key = p256::ecdsa::SigningKey::from_bytes(bytes.into())
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Invalid P-256 secret key: {e}")))?;
        Ok(Self { signing_key })
    }

    /// Get the secret key bytes (32 bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        self.signing_key.to_bytes().to_vec()
    }

    /// Get the public key.
    pub fn public_key(&self) -> P256VerifyingKey {
        P256VerifyingKey {
            verifying_key: *self.signing_key.verifying_key(),
        }
    }

    /// Sign a message using ECDSA with SHA-256 (compatible with iOS ecdsaSignatureMessageX962SHA256).
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        use p256::ecdsa::{Signature, signature::Signer};
        let signature: Signature = self.signing_key.sign(message);
        signature.to_der().as_bytes().to_vec()
    }
}

/// P-256 ECDSA public key for verifying iOS Secure Enclave signatures.
#[derive(Clone)]
pub struct P256VerifyingKey {
    verifying_key: p256::ecdsa::VerifyingKey,
}

impl P256VerifyingKey {
    /// Create from SEC1-encoded bytes (65 bytes uncompressed, 33 bytes compressed).
    pub fn from_sec1_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let verifying_key = p256::ecdsa::VerifyingKey::from_sec1_bytes(bytes)
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Invalid P-256 public key: {e}")))?;
        Ok(Self { verifying_key })
    }

    /// Create from base64-encoded SEC1 bytes.
    pub fn from_base64(s: &str) -> Result<Self, CryptoError> {
        let bytes = general_purpose::STANDARD
            .decode(s)
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Base64 decode error: {e}")))?;
        Self::from_sec1_bytes(&bytes)
    }

    /// Get as SEC1 uncompressed bytes (65 bytes).
    pub fn to_sec1_bytes(&self) -> Vec<u8> {
        self.verifying_key
            .to_encoded_point(false)
            .as_bytes()
            .to_vec()
    }

    /// Get as base64-encoded SEC1 uncompressed.
    pub fn to_base64(&self) -> String {
        general_purpose::STANDARD.encode(self.to_sec1_bytes())
    }

    /// Verify an ECDSA signature (DER or fixed-size format).
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), CryptoError> {
        use p256::ecdsa::{Signature, signature::Verifier};

        // Try DER format first (what iOS produces), then fixed-size
        let sig = Signature::from_der(signature)
            .or_else(|_| Signature::from_slice(signature))
            .map_err(|_| CryptoError::InvalidSignature)?;

        self.verifying_key
            .verify(message, &sig)
            .map_err(|_| CryptoError::VerificationFailed)
    }

    /// Convert to DID:key format for P-256 public keys.
    ///
    /// Format: `did:key:z{base58btc(0x1200 || compressed_public_key)}`
    /// - `z` = multibase prefix for base58btc
    /// - `0x1200` = multicodec prefix for P-256 public key
    pub fn to_did_key(&self) -> String {
        // P-256 public key multicodec prefix (varint encoded 0x1200)
        const P256_MULTICODEC: [u8; 2] = [0x80, 0x24];

        let pk_bytes = self
            .verifying_key
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        let mut prefixed = Vec::with_capacity(2 + pk_bytes.len());
        prefixed.extend_from_slice(&P256_MULTICODEC);
        prefixed.extend_from_slice(&pk_bytes);

        let encoded = bs58::encode(&prefixed).into_string();
        format!("did:key:z{encoded}")
    }

    /// Parse a P-256 DID:key string back to a public key.
    pub fn from_did_key(did: &str) -> Result<Self, CryptoError> {
        const P256_MULTICODEC: [u8; 2] = [0x80, 0x24];

        let encoded = did
            .strip_prefix("did:key:z")
            .ok_or_else(|| CryptoError::InvalidKeyFormat("Invalid DID:key prefix".to_string()))?;

        let decoded = bs58::decode(encoded)
            .into_vec()
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Base58 decode error: {e}")))?;

        if decoded.len() < 2 || decoded[0] != P256_MULTICODEC[0] || decoded[1] != P256_MULTICODEC[1]
        {
            return Err(CryptoError::InvalidKeyFormat(
                "Invalid P-256 multicodec prefix".to_string(),
            ));
        }

        let pk_bytes = &decoded[2..];
        Self::from_sec1_bytes(pk_bytes)
    }
}

impl fmt::Display for P256VerifyingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_base64())
    }
}

impl fmt::Debug for P256VerifyingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("P256VerifyingKey")
            .field("base64", &self.to_base64())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keypair_generation() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        assert_eq!(public_key.to_bytes().len(), 32);
    }

    #[test]
    fn test_sign_and_verify() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"test message";
        let signature = keypair.sign(message);
        assert!(public_key.verify(message, &signature).is_ok());
    }

    #[test]
    fn test_verify_wrong_message() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"test message";
        let wrong_message = b"wrong message";
        let signature = keypair.sign(message);
        assert!(public_key.verify(wrong_message, &signature).is_err());
    }

    #[test]
    fn test_base64_roundtrip() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let base64 = public_key.to_base64();
        let decoded = AgentPublicKey::from_base64(&base64).unwrap();
        assert_eq!(public_key.to_bytes(), decoded.to_bytes());
    }

    #[test]
    fn test_keypair_serialization() {
        let keypair = AgentKeypair::generate();
        let bytes = keypair.to_bytes();
        let restored = AgentKeypair::from_bytes(&bytes).unwrap();
        let message = b"test";
        let sig1 = keypair.sign(message);
        let sig2 = restored.sign(message);
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_invalid_signature_length() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"test";
        let bad_sig = vec![0u8; 32];
        assert!(matches!(
            public_key.verify(message, &bad_sig),
            Err(CryptoError::InvalidSignature)
        ));
    }

    #[test]
    fn test_did_key_format() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let did = public_key.to_did_key();

        // DID:key format should start with "did:key:z6Mk" for Ed25519 keys
        assert!(
            did.starts_with("did:key:z6Mk"),
            "DID should start with 'did:key:z6Mk', got: {}",
            did
        );
    }

    #[test]
    fn test_did_key_roundtrip() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let did = public_key.to_did_key();

        let decoded = AgentPublicKey::from_did_key(&did).expect("Failed to parse DID:key");
        assert_eq!(public_key.to_bytes(), decoded.to_bytes());
    }

    #[test]
    fn test_did_key_invalid_prefix() {
        let result = AgentPublicKey::from_did_key("invalid:key:z123");
        assert!(matches!(result, Err(CryptoError::InvalidKeyFormat(_))));
    }

    #[test]
    fn test_did_key_invalid_multicodec() {
        // Create a DID with wrong multicodec prefix
        let bad_did = "did:key:z11111111111111111111111111111111111111";
        let result = AgentPublicKey::from_did_key(bad_did);
        assert!(matches!(result, Err(CryptoError::InvalidKeyFormat(_))));
    }

    #[test]
    fn test_did_key_deterministic() {
        // Create keypair from fixed bytes
        let fixed_bytes = [42u8; 32];
        let keypair = AgentKeypair::from_bytes(&fixed_bytes).unwrap();
        let public_key = keypair.public_key();

        let did1 = public_key.to_did_key();
        let did2 = public_key.to_did_key();
        assert_eq!(did1, did2, "DID generation should be deterministic");
    }

    // TDD: iOS Secure Enclave uses P-256 ECDSA, not Ed25519
    // These tests define the expected behavior for iOS client registration

    #[test]
    fn test_p256_public_key_from_sec1_uncompressed() {
        // iOS SecKeyCopyExternalRepresentation produces 65-byte SEC1 uncompressed format
        // Format: 0x04 || x (32 bytes) || y (32 bytes)
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let sec1_bytes = public_key.to_sec1_bytes();

        assert_eq!(sec1_bytes.len(), 65, "SEC1 uncompressed should be 65 bytes");
        assert_eq!(sec1_bytes[0], 0x04, "SEC1 uncompressed starts with 0x04");

        // Roundtrip
        let restored = P256VerifyingKey::from_sec1_bytes(&sec1_bytes).unwrap();
        assert_eq!(public_key.to_sec1_bytes(), restored.to_sec1_bytes());
    }

    #[test]
    fn test_p256_sign_and_verify() {
        // iOS signs with ecdsaSignatureMessageX962SHA256
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"challenge data from server";

        let signature = keypair.sign(message);
        assert!(public_key.verify(message, &signature).is_ok());
    }

    #[test]
    fn test_p256_verify_wrong_message_fails() {
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"correct message";
        let wrong_message = b"wrong message";

        let signature = keypair.sign(message);
        assert!(public_key.verify(wrong_message, &signature).is_err());
    }

    #[test]
    fn test_p256_did_key_format() {
        // P-256 DID:key uses multicodec 0x1200 (p256-pub)
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let did = public_key.to_did_key();

        // P-256 DIDs start with "did:key:zDn" (different from Ed25519 "z6Mk")
        assert!(
            did.starts_with("did:key:zDn"),
            "P-256 DID should start with 'did:key:zDn', got: {}",
            did
        );
    }

    #[test]
    fn test_p256_did_key_roundtrip() {
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let did = public_key.to_did_key();

        let restored = P256VerifyingKey::from_did_key(&did).expect("Should parse P-256 DID:key");
        assert_eq!(public_key.to_sec1_bytes(), restored.to_sec1_bytes());
    }
}
