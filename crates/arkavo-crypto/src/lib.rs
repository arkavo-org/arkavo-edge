use base64::{Engine as _, engine::general_purpose};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
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
        let signing_key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
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

        // Validate prefix
        let encoded = did
            .strip_prefix("did:key:z")
            .ok_or_else(|| CryptoError::InvalidKeyFormat("Invalid DID:key prefix".to_string()))?;

        // Decode base58btc
        let decoded = bs58::decode(encoded)
            .into_vec()
            .map_err(|e| CryptoError::InvalidKeyFormat(format!("Base58 decode error: {}", e)))?;

        // Check multicodec prefix
        if decoded.len() < 2
            || decoded[0] != ED25519_MULTICODEC[0]
            || decoded[1] != ED25519_MULTICODEC[1]
        {
            return Err(CryptoError::InvalidKeyFormat(
                "Invalid Ed25519 multicodec prefix".to_string(),
            ));
        }

        // Extract public key bytes
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
}
