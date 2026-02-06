//! Cryptographic helper functions
//!
//! Common utilities for cipher initialization, nonce generation, etc.

use crate::types::{AesKey, Nonce96};
use aes_gcm::{Aes256Gcm, KeyInit};
use rand::{RngCore, rngs::OsRng};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Invalid key length")]
    InvalidKeyLength,

    #[error("Cipher initialization failed")]
    CipherInitFailed,
}

/// Create an AES-256-GCM cipher from a key
///
/// This helper eliminates repeated cipher initialization code.
pub fn create_aes_cipher(key: &AesKey) -> Result<Aes256Gcm, CryptoError> {
    Aes256Gcm::new_from_slice(key.as_slice()).map_err(|_| CryptoError::InvalidKeyLength)
}

/// Generate a random 96-bit nonce for AES-GCM
///
/// This helper eliminates repeated nonce generation code.
pub fn generate_nonce() -> Nonce96 {
    let mut nonce = Nonce96::default();
    OsRng.fill_bytes(nonce.as_mut_slice());
    nonce
}

/// Generate a random 32-byte key
pub fn generate_key_32() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_cipher() {
        let key_bytes = generate_key_32();
        let key = AesKey::from_slice(&key_bytes).unwrap();
        let cipher = create_aes_cipher(&key);
        assert!(cipher.is_ok());
    }

    #[test]
    fn test_generate_nonce() {
        let nonce1 = generate_nonce();
        let nonce2 = generate_nonce();
        // Nonces should be different (with overwhelming probability)
        assert_ne!(nonce1.as_slice(), nonce2.as_slice());
    }

    #[test]
    fn test_generate_key() {
        let key1 = generate_key_32();
        let key2 = generate_key_32();
        // Keys should be different
        assert_ne!(key1, key2);
    }
}
