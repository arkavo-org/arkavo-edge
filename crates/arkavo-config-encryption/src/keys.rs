use crate::error::{Error, Result};
use p256::ecdsa::{
    SigningKey, VerifyingKey, signature::SignatureEncoding, signature::Signer, signature::Verifier,
};
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
        let verifying_key = VerifyingKey::from_sec1_bytes(&self.bytes)
            .map_err(|e| Error::Signature(format!("Invalid public key: {}", e)))?;

        let sig = match p256::ecdsa::DerSignature::from_bytes(signature) {
            Ok(s) => s,
            Err(_) => return Ok(false),
        };

        match verifying_key.verify(message, &sig) {
            Ok(()) => Ok(true),
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
        let signing_key = SigningKey::from_slice(&self.bytes)
            .map_err(|e| Error::Signature(format!("Failed to create signing key: {}", e)))?;

        let signature: p256::ecdsa::DerSignature = signing_key.sign(message);

        Ok(signature.to_vec())
    }
}

pub struct KeyPair {
    pub private_key: PrivateKey,
    pub public_key: PublicKey,
}

impl KeyPair {
    pub fn generate() -> Result<Self> {
        let signing_key = SigningKey::random(&mut rand::rngs::OsRng);
        let verifying_key = VerifyingKey::from(&signing_key);

        let private_bytes = signing_key.to_bytes().to_vec();
        let public_bytes = verifying_key.to_encoded_point(false).as_bytes().to_vec();

        Ok(Self {
            private_key: PrivateKey::from_bytes(private_bytes),
            public_key: PublicKey::from_bytes(public_bytes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("TDFS-011")]
    #[test]
    fn test_generate_key_pair() {
        let key_pair = KeyPair::generate().unwrap();
        assert!(!key_pair.private_key.as_bytes().is_empty());
        assert!(!key_pair.public_key.as_bytes().is_empty());
    }

    #[spec("TDFS-011")]
    #[test]
    fn test_sign_and_verify() {
        let key_pair = KeyPair::generate().unwrap();
        let message = b"test message";

        let signature = key_pair.private_key.sign(message).unwrap();
        assert!(!signature.is_empty());

        let verified = key_pair.public_key.verify(message, &signature).unwrap();
        assert!(verified);
    }

    #[spec("TDFS-011")]
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

    #[spec("TDFS-011")]
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
