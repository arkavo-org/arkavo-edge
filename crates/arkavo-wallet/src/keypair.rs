use crate::address::EvmAddress;
use crate::error::{Result, WalletError};
use crate::signature::Signature;
use k256::ecdsa::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::fmt;
use zeroize::Zeroize;

/// EVM-compatible keypair using secp256k1
pub struct EvmKeypair {
    signing_key: SigningKey,
}

impl EvmKeypair {
    /// Generate a new random keypair
    pub fn generate() -> Self {
        let signing_key = SigningKey::random(&mut rand::thread_rng());
        Self { signing_key }
    }

    /// Create keypair from 32-byte private key
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self> {
        let signing_key = SigningKey::from_bytes(bytes.into())
            .map_err(|e| WalletError::InvalidKeyFormat(format!("Invalid private key: {e}")))?;
        Ok(Self { signing_key })
    }

    /// Create keypair from hex-encoded private key
    pub fn from_hex(hex_str: &str) -> Result<Self> {
        let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        let bytes = hex::decode(hex_str)
            .map_err(|e| WalletError::InvalidKeyFormat(format!("Invalid hex: {e}")))?;
        if bytes.len() != 32 {
            return Err(WalletError::InvalidKeyFormat(format!(
                "Expected 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);
        let result = Self::from_bytes(&key_bytes);
        key_bytes.zeroize();
        result
    }

    /// Get the private key bytes (handle with care)
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes().into()
    }

    /// Get the public key
    pub fn public_key(&self) -> EvmPublicKey {
        EvmPublicKey {
            verifying_key: *self.signing_key.verifying_key(),
        }
    }

    /// Get the EVM address derived from this keypair
    pub fn address(&self) -> EvmAddress {
        self.public_key().address()
    }

    /// Sign a message hash (32 bytes, already hashed)
    ///
    /// # Panics
    /// This function should not panic with a valid key.
    pub fn sign_prehash(&self, hash: &[u8; 32]) -> Signature {
        let (sig, recovery_id) = self
            .signing_key
            .sign_prehash_recoverable(hash)
            .expect("signing should not fail with valid key");
        Signature::from_k256(&sig, recovery_id)
    }

    /// Sign a message (will be hashed with Keccak256 and prefixed per EIP-191)
    pub fn sign_message(&self, message: &[u8]) -> Signature {
        let hash = Self::eth_message_hash(message);
        self.sign_prehash(&hash)
    }

    /// Compute Ethereum signed message hash (EIP-191)
    pub fn eth_message_hash(message: &[u8]) -> [u8; 32] {
        let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
        let mut hasher = Keccak256::new();
        hasher.update(prefix.as_bytes());
        hasher.update(message);
        hasher.finalize().into()
    }

    /// Convert to did:key format using secp256k1 multicodec (0xe701)
    pub fn to_did_key(&self) -> String {
        self.public_key().to_did_key()
    }
}

impl Drop for EvmKeypair {
    fn drop(&mut self) {
        // SigningKey implements Zeroize internally
    }
}

impl fmt::Debug for EvmKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EvmKeypair({})", self.address())
    }
}

/// EVM public key (uncompressed secp256k1)
#[derive(Clone)]
pub struct EvmPublicKey {
    verifying_key: VerifyingKey,
}

impl EvmPublicKey {
    /// Create from uncompressed public key bytes (65 bytes with 0x04 prefix, or 64 without)
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let verifying_key = if bytes.len() == 65 && bytes[0] == 0x04 {
            VerifyingKey::from_sec1_bytes(bytes)
        } else if bytes.len() == 64 {
            let mut prefixed = vec![0x04];
            prefixed.extend_from_slice(bytes);
            VerifyingKey::from_sec1_bytes(&prefixed)
        } else if bytes.len() == 33 {
            VerifyingKey::from_sec1_bytes(bytes)
        } else {
            return Err(WalletError::InvalidKeyFormat(format!(
                "Invalid public key length: {}",
                bytes.len()
            )));
        }
        .map_err(|e| WalletError::InvalidKeyFormat(format!("Invalid public key: {e}")))?;

        Ok(Self { verifying_key })
    }

    /// Get uncompressed public key bytes (64 bytes, without 0x04 prefix)
    pub fn to_bytes(&self) -> [u8; 64] {
        let encoded = self.verifying_key.to_encoded_point(false);
        let bytes = encoded.as_bytes();
        let mut result = [0u8; 64];
        result.copy_from_slice(&bytes[1..65]);
        result
    }

    /// Get compressed public key bytes (33 bytes)
    pub fn to_compressed_bytes(&self) -> [u8; 33] {
        let encoded = self.verifying_key.to_encoded_point(true);
        let bytes = encoded.as_bytes();
        let mut result = [0u8; 33];
        result.copy_from_slice(bytes);
        result
    }

    /// Derive EVM address from this public key
    ///
    /// # Panics
    /// This function should not panic with a valid public key.
    pub fn address(&self) -> EvmAddress {
        EvmAddress::from_public_key(&self.to_bytes())
            .expect("valid public key should produce valid address")
    }

    /// Verify a signature against a message hash
    pub fn verify_prehash(&self, hash: &[u8; 32], signature: &Signature) -> Result<()> {
        let recovered = signature.recover_public_key(hash)?;
        if recovered == self.verifying_key {
            Ok(())
        } else {
            Err(WalletError::VerificationFailed)
        }
    }

    /// Convert to did:key format using secp256k1 multicodec (0xe701)
    pub fn to_did_key(&self) -> String {
        let compressed = self.to_compressed_bytes();
        let mut multicodec = vec![0xe7, 0x01];
        multicodec.extend_from_slice(&compressed);
        let encoded = bs58::encode(&multicodec).into_string();
        format!("did:key:z{encoded}")
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }
}

impl fmt::Display for EvmPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", self.to_hex())
    }
}

impl fmt::Debug for EvmPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EvmPublicKey({})", self.address())
    }
}

impl Serialize for EvmPublicKey {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for EvmPublicKey {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        Self::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keypair_generate() {
        let kp = EvmKeypair::generate();
        let addr = kp.address();
        assert_eq!(addr.as_bytes().len(), 20);
    }

    #[test]
    fn test_keypair_from_bytes() {
        let bytes = [1u8; 32];
        let kp = EvmKeypair::from_bytes(&bytes).unwrap();
        let kp2 = EvmKeypair::from_bytes(&bytes).unwrap();
        assert_eq!(kp.address(), kp2.address());
    }

    #[test]
    fn test_keypair_from_hex() {
        let hex = "0x0000000000000000000000000000000000000000000000000000000000000001";
        let kp = EvmKeypair::from_hex(hex).unwrap();
        assert!(!kp.address().to_hex().is_empty());
    }

    #[test]
    fn test_sign_and_verify() {
        let kp = EvmKeypair::generate();
        let message = b"hello world";
        let sig = kp.sign_message(message);
        let hash = EvmKeypair::eth_message_hash(message);
        kp.public_key().verify_prehash(&hash, &sig).unwrap();
    }

    #[test]
    fn test_sign_prehash() {
        let kp = EvmKeypair::generate();
        let hash = [0xab; 32];
        let sig = kp.sign_prehash(&hash);
        kp.public_key().verify_prehash(&hash, &sig).unwrap();
    }

    #[test]
    fn test_public_key_roundtrip() {
        let kp = EvmKeypair::generate();
        let pk = kp.public_key();
        let bytes = pk.to_bytes();
        let pk2 = EvmPublicKey::from_bytes(&bytes).unwrap();
        assert_eq!(pk.address(), pk2.address());
    }

    #[test]
    fn test_did_key() {
        let kp = EvmKeypair::generate();
        let did = kp.to_did_key();
        assert!(did.starts_with("did:key:z"));
    }

    #[test]
    fn test_compressed_public_key() {
        let kp = EvmKeypair::generate();
        let compressed = kp.public_key().to_compressed_bytes();
        assert_eq!(compressed.len(), 33);
        assert!(compressed[0] == 0x02 || compressed[0] == 0x03);
    }
}
