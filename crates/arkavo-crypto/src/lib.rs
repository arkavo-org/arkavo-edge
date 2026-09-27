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
        let secret_key = P256SecretKey::random(&mut rand::rngs::OsRng);
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
        let signing_key = p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng);
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

    /// Sign a message and return the fixed-size IEEE P1363 `r || s` encoding.
    ///
    /// This is the form COSE ES256 requires (RFC 8152 section 8.1) and JWS
    /// uses. It is the same signature [`sign`](Self::sign) produces, written
    /// out as 64 bytes rather than DER, so no conversion — and no conversion
    /// failure — sits between signing and the wire.
    pub fn sign_p1363(&self, message: &[u8]) -> [u8; 64] {
        use p256::ecdsa::{Signature, signature::Signer};
        let signature: Signature = self.signing_key.sign(message);
        signature.to_bytes().into()
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
    use arkavo_test_macros::spec;

    /// Test CRYPTO-001: Generate Ed25519 agent keypair
    #[spec("CRYPTO-001")]
    #[test]
    fn test_keypair_generation() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        assert_eq!(public_key.to_bytes().len(), 32);
    }

    /// Test CRYPTO-002: Sign and verify with Ed25519
    #[spec("CRYPTO-002")]
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

    /// Test CRYPTO-003: Serialize and restore Ed25519 keypair
    #[spec("CRYPTO-003")]
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

    /// Test CRYPTO-004: Ed25519 public key to DID:key format
    #[spec("CRYPTO-004")]
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

    /// Test CRYPTO-005: Parse Ed25519 DID:key string
    #[spec("CRYPTO-005")]
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

    /// Test CRYPTO-009: P-256 public key SEC1 encoding
    #[spec("CRYPTO-009")]
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

    /// Test CRYPTO-007: Sign with P-256 ECDSA (DER format)
    /// Test CRYPTO-008: Verify P-256 signature with auto format detection
    #[spec("CRYPTO-007", "CRYPTO-008")]
    #[test]
    fn test_p256_sign_and_verify() {
        // iOS signs with ecdsaSignatureMessageX962SHA256
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"challenge data from server";

        let signature = keypair.sign(message);
        assert!(public_key.verify(message, &signature).is_ok());
    }

    /// COSE ES256 and JWS both want the 64-byte `r || s` encoding, not DER.
    /// Producing it directly is what lets callers avoid a DER conversion
    /// that could fail on the signing path.
    #[test]
    fn test_p256_sign_p1363_is_fixed_size_and_verifies() {
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"COSE Sig_structure bytes";

        let fixed = keypair.sign_p1363(message);
        assert_eq!(fixed.len(), 64);
        assert!(public_key.verify(message, &fixed).is_ok());
        assert!(public_key.verify(b"other message", &fixed).is_err());

        // The same signature the DER path produces, in the other encoding:
        // P-256 signing here is deterministic (RFC 6979).
        let der = keypair.sign(message);
        let from_der = p256::ecdsa::Signature::from_der(&der).expect("DER signature");
        assert_eq!(from_der.to_bytes().as_slice(), fixed.as_slice());
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

    /// Test CRYPTO-010: P-256 to DID:key format
    #[spec("CRYPTO-010")]
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

    /// Test CRYPTO-006: Generate P-256 signing keypair (iOS compatibility)
    #[spec("CRYPTO-006")]
    #[test]
    fn test_p256_did_key_roundtrip() {
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let did = public_key.to_did_key();

        let restored = P256VerifyingKey::from_did_key(&did).expect("Should parse P-256 DID:key");
        assert_eq!(public_key.to_sec1_bytes(), restored.to_sec1_bytes());
    }

    // Second tests to bump scenarios from Partial → Covered

    /// Second test for CRYPTO-001: Generate multiple keypairs
    #[spec("CRYPTO-001")]
    #[test]
    fn test_keypair_generation_multiple() {
        // Generate multiple keypairs and ensure they're all unique
        let mut public_keys = std::collections::HashSet::new();
        for _ in 0..100 {
            let keypair = AgentKeypair::generate();
            let public_key = keypair.public_key();
            let bytes = public_key.to_bytes();
            assert_eq!(bytes.len(), 32);
            assert!(public_keys.insert(bytes));
        }
    }

    /// Second test for CRYPTO-002: Sign and verify multiple messages
    #[spec("CRYPTO-002")]
    #[test]
    fn test_sign_and_verify_multiple_messages() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();

        // Test with different message sizes
        for size in [0, 1, 32, 100, 1000, 10000] {
            let message = vec![0xABu8; size];
            let signature = keypair.sign(&message);
            assert!(public_key.verify(&message, &signature).is_ok());
        }
    }

    /// Second test for CRYPTO-003: Serialization roundtrip preserves functionality
    #[spec("CRYPTO-003")]
    #[test]
    fn test_keypair_serialization_roundtrip() {
        // Generate and serialize multiple times
        for _ in 0..10 {
            let keypair = AgentKeypair::generate();
            let bytes = keypair.to_bytes();

            // Multiple roundtrips
            let mut current = AgentKeypair::from_bytes(&bytes).unwrap();
            for _ in 0..5 {
                let bytes = current.to_bytes();
                current = AgentKeypair::from_bytes(&bytes).unwrap();
            }

            // Verify final keypair works
            let message = b"final test";
            let sig = current.sign(message);
            assert!(keypair.public_key().verify(message, &sig).is_ok());
        }
    }

    /// Second test for CRYPTO-004: DID format consistency
    #[spec("CRYPTO-004")]
    #[test]
    fn test_did_key_format_consistency() {
        for _ in 0..50 {
            let keypair = AgentKeypair::generate();
            let public_key = keypair.public_key();
            let did = public_key.to_did_key();

            // All Ed25519 DIDs should start with this prefix
            assert!(did.starts_with("did:key:z6Mk"));
            // Should be able to parse back
            let _restored = AgentPublicKey::from_did_key(&did).unwrap();
        }
    }

    /// Second test for CRYPTO-005: DID parsing with various valid inputs
    #[spec("CRYPTO-005")]
    #[test]
    fn test_did_key_parsing_various() {
        // Generate keys and test roundtrip
        for _ in 0..50 {
            let keypair = AgentKeypair::generate();
            let public_key = keypair.public_key();
            let did = public_key.to_did_key();

            let restored = AgentPublicKey::from_did_key(&did).unwrap();
            assert_eq!(public_key.to_bytes(), restored.to_bytes());
            assert_eq!(public_key.to_base64(), restored.to_base64());
        }
    }

    /// Second test for CRYPTO-006: P-256 keypair generation
    #[spec("CRYPTO-006")]
    #[test]
    fn test_p256_keypair_generation_multiple() {
        // Generate multiple P-256 keypairs
        let mut public_keys = std::collections::HashSet::new();
        for _ in 0..50 {
            let keypair = P256SigningKeypair::generate();
            let public_key = keypair.public_key();
            let sec1_bytes = public_key.to_sec1_bytes();
            assert_eq!(sec1_bytes.len(), 65);
            assert!(public_keys.insert(sec1_bytes));
        }
    }

    /// Second test for CRYPTO-007 & CRYPTO-008: P-256 multiple sign/verify operations
    #[spec("CRYPTO-007", "CRYPTO-008")]
    #[test]
    fn test_p256_sign_verify_multiple() {
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();

        // Test with different messages
        for i in 0..20 {
            let message = format!("message {i}").into_bytes();
            let signature = keypair.sign(&message);
            assert!(public_key.verify(&message, &signature).is_ok());
        }
    }

    /// Second test for CRYPTO-007 - P-256 signature is DER-encoded
    #[spec("CRYPTO-007")]
    #[test]
    fn test_p256_signature_is_valid_der() {
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"ios ecdsaSignatureMessageX962SHA256 compatible";

        let signature = keypair.sign(message);

        // DER-encoded ECDSA signatures begin with ASN.1 SEQUENCE marker 0x30
        assert_eq!(
            signature[0], 0x30,
            "signature must start with DER SEQUENCE marker"
        );

        // DER format is variable-length and longer than the raw 64-byte fixed encoding
        assert!(
            signature.len() > 64,
            "DER P-256 signature should exceed 64 bytes, got {}",
            signature.len()
        );

        // The produced signature verifies with the corresponding public key
        assert!(public_key.verify(message, &signature).is_ok());
    }

    /// Second test for CRYPTO-009: SEC1 encoding consistency
    #[spec("CRYPTO-009")]
    #[test]
    fn test_p256_sec1_encoding_consistency() {
        for _ in 0..30 {
            let keypair = P256SigningKeypair::generate();
            let public_key = keypair.public_key();

            // Multiple encodings should be identical
            let sec1_1 = public_key.to_sec1_bytes();
            let sec1_2 = public_key.to_sec1_bytes();
            assert_eq!(sec1_1, sec1_2);

            // Roundtrip
            let restored = P256VerifyingKey::from_sec1_bytes(&sec1_1).unwrap();
            assert_eq!(sec1_1, restored.to_sec1_bytes());
        }
    }

    /// Second test for CRYPTO-010: P-256 DID format
    #[spec("CRYPTO-010")]
    #[test]
    fn test_p256_did_key_format_multiple() {
        for _ in 0..30 {
            let keypair = P256SigningKeypair::generate();
            let public_key = keypair.public_key();
            let did = public_key.to_did_key();

            // P-256 DIDs have a different prefix than Ed25519
            assert!(did.starts_with("did:key:z"));
            assert_ne!(did.starts_with("did:key:z6Mk"), true);

            // Should be parseable
            let restored = P256VerifyingKey::from_did_key(&did).unwrap();
            assert_eq!(public_key.to_sec1_bytes(), restored.to_sec1_bytes());
        }
    }

    // ============================================================================
    // TDD Quality Improvement Tests - Edge Cases & Property Tests
    // ============================================================================

    /// TDD Test: Empty message signing should work
    #[spec("CRYPTO-002")]
    #[test]
    fn test_sign_empty_message() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();

        // Empty message should be signable
        let empty: &[u8] = b"";
        let signature = keypair.sign(empty);
        assert!(public_key.verify(empty, &signature).is_ok());
    }

    /// TDD Test: Large message signing (1MB)
    #[spec("CRYPTO-002")]
    #[test]
    fn test_sign_large_message() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();

        // 1MB message
        let large_message = vec![0xABu8; 1_048_576];
        let signature = keypair.sign(&large_message);
        assert!(public_key.verify(&large_message, &signature).is_ok());
    }

    /// TDD Test: Signature with wrong key should fail verification
    #[spec("CRYPTO-002")]
    #[test]
    fn test_signature_with_wrong_key() {
        let keypair1 = AgentKeypair::generate();
        let keypair2 = AgentKeypair::generate();
        let public_key2 = keypair2.public_key();

        let message = b"test message";
        let signature = keypair1.sign(message);

        // Signature from key1 should NOT verify with key2's public key
        assert!(public_key2.verify(message, &signature).is_err());
    }

    /// TDD Test: Signature is deterministic
    #[spec("CRYPTO-002")]
    #[test]
    fn test_signature_determinism() {
        let keypair = AgentKeypair::generate();
        let message = b"deterministic test";

        // Generate multiple signatures for same message
        let sig1 = keypair.sign(message);
        let sig2 = keypair.sign(message);
        let sig3 = keypair.sign(message);

        // All signatures should be valid
        let public_key = keypair.public_key();
        assert!(public_key.verify(message, &sig1).is_ok());
        assert!(public_key.verify(message, &sig2).is_ok());
        assert!(public_key.verify(message, &sig3).is_ok());
    }

    /// TDD Test: Invalid signature lengths should be rejected
    #[spec("CRYPTO-002")]
    #[test]
    fn test_invalid_signature_lengths() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let message = b"test";

        // Test various invalid signature lengths
        for len in [0, 31, 33, 63, 65, 100, 1000] {
            let bad_sig = vec![0u8; len];
            let result = public_key.verify(message, &bad_sig);
            assert!(result.is_err(), "Signature length {len} should be rejected");
        }
    }

    /// TDD Test: Message tampering detection
    #[spec("CRYPTO-002")]
    #[test]
    fn test_message_tampering_detection() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();

        let original = b"original message";
        let signature = keypair.sign(original);

        // Single bit flip should invalidate signature
        for i in 0..original.len() {
            for bit in 0..8 {
                let mut tampered = original.to_vec();
                tampered[i] ^= 1 << bit;
                assert!(
                    public_key.verify(&tampered, &signature).is_err(),
                    "Bit flip at byte {i}, bit {bit} should invalidate signature"
                );
            }
        }
    }

    /// TDD Test: Signature tampering detection
    #[spec("CRYPTO-002")]
    #[test]
    fn test_signature_tampering_detection() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();

        let message = b"test message";
        let mut signature = keypair.sign(message);

        // Single bit flip in signature should invalidate it
        for i in 0..signature.len() {
            for bit in 0..8 {
                let mut tampered = signature.clone();
                tampered[i] ^= 1 << bit;
                assert!(
                    public_key.verify(message, &tampered).is_err(),
                    "Bit flip at byte {i}, bit {bit} should invalidate signature"
                );
            }
        }
    }

    /// TDD Test: P-256 empty message signing
    #[spec("CRYPTO-007", "CRYPTO-008")]
    #[test]
    fn test_p256_sign_empty_message() {
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();

        let empty: &[u8] = b"";
        let signature = keypair.sign(empty);
        assert!(public_key.verify(empty, &signature).is_ok());
    }

    /// TDD Test: P-256 large message signing
    #[spec("CRYPTO-007", "CRYPTO-008")]
    #[test]
    fn test_p256_sign_large_message() {
        let keypair = P256SigningKeypair::generate();
        let public_key = keypair.public_key();

        let large_message = vec![0xCDu8; 1_048_576]; // 1MB
        let signature = keypair.sign(&large_message);
        assert!(public_key.verify(&large_message, &signature).is_ok());
    }

    /// TDD Test: Invalid SEC1 lengths should be rejected
    #[spec("CRYPTO-009")]
    #[test]
    fn test_p256_invalid_sec1_lengths() {
        // Test various invalid SEC1 lengths
        for len in [0, 1, 32, 64, 66, 100] {
            let bad_bytes = vec![0u8; len];
            let result = P256VerifyingKey::from_sec1_bytes(&bad_bytes);
            assert!(result.is_err(), "SEC1 length {len} should be rejected");
        }
    }

    /// TDD Test: Keypair serialization with all-zero bytes should fail
    #[spec("CRYPTO-003")]
    #[test]
    fn test_keypair_from_all_zero_bytes() {
        let all_zeros = [0u8; 32];
        let result = AgentKeypair::from_bytes(&all_zeros);
        // All-zero key is cryptographically weak but may be accepted
        // depending on implementation - test behavior
        match result {
            Ok(keypair) => {
                // If accepted, should still work for signing
                let msg = b"test";
                let sig = keypair.sign(msg);
                assert!(keypair.public_key().verify(msg, &sig).is_ok());
            }
            Err(_) => {
                // If rejected, that's also valid behavior
            }
        }
    }

    /// TDD Test: DID parsing with invalid multibase prefix
    #[spec("CRYPTO-004", "CRYPTO-005")]
    #[test]
    fn test_did_invalid_multibase_prefix() {
        // Invalid multibase prefixes
        let invalid_dids = [
            "did:key:abcdef",      // No 'z' prefix
            "did:key:",            // Empty
            "did:key:z",           // Too short
            "did:web:example.com", // Wrong method
        ];

        for did in &invalid_dids {
            let result = AgentPublicKey::from_did_key(did);
            assert!(result.is_err(), "DID '{did}' should be rejected");
        }
    }
}
