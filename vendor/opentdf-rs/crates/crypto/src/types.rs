//! Zeroizing cryptographic key types
//!
//! This module provides secure wrappers for cryptographic key material that
//! automatically clear memory on drop to prevent key leakage.

use zeroize::{Zeroize, ZeroizeOnDrop};

#[cfg(feature = "kem-ec")]
use zeroize::Zeroizing;

/// AES-256 key (32 bytes) that zeroizes on drop
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct AesKey(pub(crate) [u8; 32]);

impl AesKey {
    /// Create a new AES key from a 32-byte slice
    pub fn from_slice(bytes: &[u8]) -> Result<Self, KeyError> {
        if bytes.len() != 32 {
            return Err(KeyError::InvalidLength {
                expected: 32,
                got: bytes.len(),
            });
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(bytes);
        Ok(AesKey(key))
    }

    /// Get a reference to the key bytes
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Get a mutable reference to the key bytes
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

/// Policy key (32 bytes) that zeroizes on drop
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct PolicyKey(pub(crate) [u8; 32]);

impl PolicyKey {
    /// Create a new policy key from a 32-byte slice
    pub fn from_slice(bytes: &[u8]) -> Result<Self, KeyError> {
        if bytes.len() != 32 {
            return Err(KeyError::InvalidLength {
                expected: 32,
                got: bytes.len(),
            });
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(bytes);
        Ok(PolicyKey(key))
    }

    /// Get a reference to the key bytes
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Get a mutable reference to the key bytes
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

/// Payload key (32 bytes) that zeroizes on drop
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct PayloadKey(pub(crate) [u8; 32]);

impl PayloadKey {
    /// Create a new payload key from a 32-byte slice
    pub fn from_slice(bytes: &[u8]) -> Result<Self, KeyError> {
        if bytes.len() != 32 {
            return Err(KeyError::InvalidLength {
                expected: 32,
                got: bytes.len(),
            });
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(bytes);
        Ok(PayloadKey(key))
    }

    /// Get a reference to the key bytes
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Get a mutable reference to the key bytes
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

/// 96-bit nonce (12 bytes) that zeroizes on drop
#[derive(Clone, Zeroize, ZeroizeOnDrop, Default)]
pub struct Nonce96(pub(crate) [u8; 12]);

impl Nonce96 {
    /// Create a new nonce from a 12-byte slice
    pub fn from_slice(bytes: &[u8]) -> Result<Self, KeyError> {
        if bytes.len() != 12 {
            return Err(KeyError::InvalidLength {
                expected: 12,
                got: bytes.len(),
            });
        }
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(bytes);
        Ok(Nonce96(nonce))
    }

    /// Get a reference to the nonce bytes
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Get a mutable reference to the nonce bytes
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

/// EC private key that zeroizes on drop (for NanoTDF)
#[cfg(feature = "kem-ec")]
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct EcPrivateKey(pub(crate) Zeroizing<Vec<u8>>);

#[cfg(feature = "kem-ec")]
impl EcPrivateKey {
    /// Create from P-256 secret key
    pub fn from_p256(key: &p256::SecretKey) -> Self {
        use p256::pkcs8::EncodePrivateKey;
        let der = key.to_pkcs8_der().expect("Failed to encode key");
        EcPrivateKey(Zeroizing::new(der.to_bytes().to_vec()))
    }

    /// Get the bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Key-related errors
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("Invalid key length: expected {expected}, got {got}")]
    InvalidLength { expected: usize, got: usize },

    #[error("Key generation failed")]
    GenerationFailed,
}
