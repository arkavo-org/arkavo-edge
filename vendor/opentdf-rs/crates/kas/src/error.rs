//! KAS server error types

use thiserror::Error;

/// Errors that can occur during KAS server operations
#[derive(Debug, Error)]
pub enum KasServerError {
    /// Invalid public key format or data
    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),

    /// Invalid private key format or data
    #[error("invalid private key: {0}")]
    InvalidPrivateKey(String),

    /// ECDH key agreement failed
    #[error("ECDH key agreement failed: {0}")]
    EcdhError(String),

    /// HKDF key derivation failed
    #[error("HKDF key derivation failed: {0}")]
    HkdfError(String),

    /// AES-GCM encryption/decryption failed
    #[error("AES-GCM operation failed: {0}")]
    AesGcmError(String),

    /// RSA decryption failed
    #[error("RSA decryption failed: {0}")]
    RsaError(String),

    /// Base64 encoding/decoding failed
    #[error("base64 error: {0}")]
    Base64Error(#[from] base64::DecodeError),

    /// Invalid NanoTDF header
    #[error("invalid NanoTDF header: {0}")]
    InvalidHeader(String),

    /// Key not configured
    #[error("key not configured: {0}")]
    KeyNotConfigured(String),

    /// Invalid key size
    #[error("invalid key size: expected {expected}, got {got}")]
    InvalidKeySize { expected: usize, got: usize },
}

impl From<p256::elliptic_curve::Error> for KasServerError {
    fn from(e: p256::elliptic_curve::Error) -> Self {
        KasServerError::InvalidPublicKey(e.to_string())
    }
}

impl From<hkdf::InvalidLength> for KasServerError {
    fn from(e: hkdf::InvalidLength) -> Self {
        KasServerError::HkdfError(e.to_string())
    }
}

impl From<aes_gcm::Error> for KasServerError {
    fn from(_: aes_gcm::Error) -> Self {
        KasServerError::AesGcmError("encryption/decryption failed".to_string())
    }
}
