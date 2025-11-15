use crate::error::{Error, Result};
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as RingKeyPair};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicKey {
    bytes: Vec<u8>,
}

impl PublicKey {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool> {
        use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};

        let public_key = UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &self.bytes);

        match public_key.verify(message, signature) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrivateKey {
    bytes: Vec<u8>,
}

impl PrivateKey {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        let rng = ring::rand::SystemRandom::new();
        let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &self.bytes, &rng)
            .map_err(|e| Error::Signature(format!("Failed to create key pair: {}", e)))?;

        let signature = key_pair
            .sign(&rng, message)
            .map_err(|e| Error::Signature(format!("Failed to sign: {}", e)))?;

        Ok(signature.as_ref().to_vec())
    }
}

pub struct KeyPair {
    pub private_key: PrivateKey,
    pub public_key: PublicKey,
}

impl KeyPair {
    pub fn generate() -> Result<Self> {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8_bytes = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
            .map_err(|e| Error::KeyGeneration(format!("Failed to generate key pair: {}", e)))?;

        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8_bytes.as_ref(), &rng)
                .map_err(|e| Error::KeyGeneration(format!("Failed to parse key pair: {}", e)))?;

        let public_key_bytes = key_pair.public_key().as_ref().to_vec();

        Ok(Self {
            private_key: PrivateKey::from_bytes(pkcs8_bytes.as_ref().to_vec()),
            public_key: PublicKey::from_bytes(public_key_bytes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_key_pair() {
        let key_pair = KeyPair::generate().unwrap();
        assert!(!key_pair.private_key.as_bytes().is_empty());
        assert!(!key_pair.public_key.as_bytes().is_empty());
    }

    #[test]
    fn test_sign_and_verify() {
        let key_pair = KeyPair::generate().unwrap();
        let message = b"test message";

        let signature = key_pair.private_key.sign(message).unwrap();
        assert!(!signature.is_empty());

        let verified = key_pair.public_key.verify(message, &signature).unwrap();
        assert!(verified);
    }

    #[test]
    fn test_verify_invalid_signature() {
        let key_pair = KeyPair::generate().unwrap();
        let message = b"test message";
        let wrong_signature = vec![0u8; 64];

        let verified = key_pair
            .public_key
            .verify(message, &wrong_signature)
            .unwrap();
        assert!(!verified);
    }

    #[test]
    fn test_verify_wrong_message() {
        let key_pair = KeyPair::generate().unwrap();
        let message = b"test message";
        let wrong_message = b"wrong message";

        let signature = key_pair.private_key.sign(message).unwrap();
        let verified = key_pair
            .public_key
            .verify(wrong_message, &signature)
            .unwrap();
        assert!(!verified);
    }
}
