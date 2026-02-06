//! Key Encapsulation Mechanisms (KEM)
//!
//! This module provides abstractions for key wrapping and unwrapping,
//! supporting both classical (RSA, EC) and future post-quantum algorithms.

use thiserror::Error;

#[cfg(feature = "kem-rsa")]
pub mod rsa;

#[cfg(feature = "kem-ec")]
pub mod ec;

pub mod pqc;

/// KEM-related errors
#[derive(Debug, Error)]
pub enum KemError {
    #[error("Key wrapping failed: {0}")]
    WrapError(String),

    #[error("Key unwrapping failed: {0}")]
    UnwrapError(String),

    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    #[error("Invalid public key")]
    InvalidPublicKey,

    #[error("Invalid private key")]
    InvalidPrivateKey,

    #[error("Key derivation failed")]
    KeyDerivationFailed,

    #[error("Unsupported algorithm: {0}")]
    UnsupportedAlgorithm(String),

    #[error("Encoding error: {0}")]
    EncodingError(String),
}

/// Trait for key encapsulation mechanisms
///
/// This abstraction supports classical (RSA-OAEP, ECDH) and future
/// post-quantum (ML-KEM) key encapsulation schemes.
pub trait KeyEncapsulation {
    /// Public key type
    type PublicKey;

    /// Private key type
    type PrivateKey;

    /// Wrapped key type (ciphertext)
    type WrappedKey;

    /// Wrap a symmetric key with a public key
    fn wrap(&self, key: &[u8], public_key: &Self::PublicKey) -> Result<Self::WrappedKey, KemError>;

    /// Unwrap a symmetric key with a private key
    fn unwrap(
        &self,
        wrapped: &Self::WrappedKey,
        private_key: &Self::PrivateKey,
    ) -> Result<Vec<u8>, KemError>;
}
