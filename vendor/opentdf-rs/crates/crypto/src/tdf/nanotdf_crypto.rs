//! NanoTDF Cryptographic Operations
//!
//! This module implements NanoTDF-specific encryption with:
//! - Variable-length GCM tags (96-128 bits currently, 64-bit support planned)
//! - 3-byte IVs (24 bits)
//! - GMAC policy binding
//! - ECDSA signature support
//!
//! ## Current Limitation
//! **64-bit GCM tags**: The NanoTDF spec default of 64-bit tags is not yet supported.
//! RustCrypto's aes-gcm crate only supports 96-128 bit tags due to a sealed trait limitation.
//!
//! ## Roadmap
//! - ✅ 96-128 bit GCM tags (current, using RustCrypto)
//! - 🚧 64-bit GCM tags via Mbed TLS backend (in development, use `nanotdf-mbedtls` feature when ready)
//! - Use 96-bit tags for now, which provides good security while we complete 64-bit support

use crate::types::AesKey;
use rand::RngCore;
use thiserror::Error;

// Conditional imports based on backend
#[cfg(not(feature = "nanotdf-mbedtls"))]
use aes_gcm::{
    Aes256Gcm as Aes256Gcm128, // Standard 128-bit tag
    aead::{Aead, KeyInit, Payload},
};
#[cfg(not(feature = "nanotdf-mbedtls"))]
use typenum::{U12, U13, U14, U15};

#[cfg(feature = "nanotdf-mbedtls")]
use mbedtls::cipher::{Cipher, Decryption, Encryption, Fresh};

/// NanoTDF encryption errors
#[derive(Debug, Error)]
pub enum NanoTdfCryptoError {
    #[error("Encryption failed")]
    EncryptionFailed,

    #[error("Decryption failed")]
    DecryptionFailed,

    #[error("Invalid IV length: expected 3 bytes, got {0}")]
    InvalidIvLength(usize),

    #[error("Invalid tag length: expected {expected}, got {actual}")]
    InvalidTagLength { expected: usize, actual: usize },

    #[error("Invalid key length")]
    InvalidKeyLength,

    #[error("Payload too large: {0} bytes (max 16777215)")]
    PayloadTooLarge(usize),
}

/// Tag size for AES-256-GCM
///
/// Without `nanotdf-mbedtls` feature: Only 96-128 bit tags supported (RustCrypto limitation)
/// With `nanotdf-mbedtls` feature: Full 64-128 bit tag support via Mbed TLS
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagSize {
    /// 64-bit tag (8 bytes) - Only available with `nanotdf-mbedtls` feature
    #[cfg(feature = "nanotdf-mbedtls")]
    Bits64 = 8,
    /// 96-bit tag (12 bytes)
    Bits96 = 12,
    /// 104-bit tag (13 bytes)
    Bits104 = 13,
    /// 112-bit tag (14 bytes)
    Bits112 = 14,
    /// 120-bit tag (15 bytes)
    Bits120 = 15,
    /// 128-bit tag (16 bytes) - standard GCM
    Bits128 = 16,
}

impl TagSize {
    /// Get tag size in bytes
    pub fn bytes(self) -> usize {
        self as usize
    }

    /// Get tag size in bits
    pub fn bits(self) -> usize {
        self.bytes() * 8
    }

    /// Create from byte count
    pub fn from_bytes(bytes: usize) -> Result<Self, NanoTdfCryptoError> {
        match bytes {
            #[cfg(feature = "nanotdf-mbedtls")]
            8 => Ok(TagSize::Bits64),
            12 => Ok(TagSize::Bits96),
            13 => Ok(TagSize::Bits104),
            14 => Ok(TagSize::Bits112),
            15 => Ok(TagSize::Bits120),
            16 => Ok(TagSize::Bits128),
            _ => Err(NanoTdfCryptoError::InvalidTagLength {
                expected: 16,
                actual: bytes,
            }),
        }
    }
}

/// NanoTDF IV (3 bytes)
#[derive(Debug, Clone, Copy)]
pub struct NanoTdfIv([u8; 3]);

impl NanoTdfIv {
    /// Create a new random IV
    pub fn random() -> Self {
        let mut rng = rand::thread_rng();
        let mut iv = [0u8; 3];
        rng.fill_bytes(&mut iv);
        Self(iv)
    }

    /// Create from bytes
    pub fn from_bytes(bytes: [u8; 3]) -> Self {
        Self(bytes)
    }

    /// Create from slice
    pub fn from_slice(bytes: &[u8]) -> Result<Self, NanoTdfCryptoError> {
        if bytes.len() != 3 {
            return Err(NanoTdfCryptoError::InvalidIvLength(bytes.len()));
        }
        let mut iv = [0u8; 3];
        iv.copy_from_slice(bytes);
        Ok(Self(iv))
    }

    /// Get as bytes
    pub fn as_bytes(&self) -> &[u8; 3] {
        &self.0
    }

    /// Convert to 12-byte nonce for AES-GCM (pad with 9 zeros prefix)
    /// Per NanoTDF spec and otdfctl implementation: [9 zero bytes][3-byte IV]
    pub fn to_gcm_nonce(&self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[9..12].copy_from_slice(&self.0); // Place IV at end, zeros at start
        nonce
    }

    /// Reserved IV for encrypted policy (must not be reused)
    pub const POLICY_IV: Self = Self([0x00, 0x00, 0x00]);
}

// Type aliases for different tag sizes using AES-256
// Note: aes-gcm crate only supports 96-128 bit tags (12-16 bytes)
// 64-bit tags are NOT supported by RustCrypto aes-gcm
type Aes256Gcm96 = aes_gcm::AesGcm<aes::Aes256, U12, U12>; // 12-byte nonce, 12-byte tag (96-bit)
type Aes256Gcm104 = aes_gcm::AesGcm<aes::Aes256, U12, U13>; // 12-byte nonce, 13-byte tag (104-bit)
type Aes256Gcm112 = aes_gcm::AesGcm<aes::Aes256, U12, U14>; // 12-byte nonce, 14-byte tag (112-bit)
type Aes256Gcm120 = aes_gcm::AesGcm<aes::Aes256, U12, U15>; // 12-byte nonce, 15-byte tag (120-bit)
// Aes256Gcm128 imported above as the standard variant (12-byte nonce, 16-byte tag, 128-bit)

/// Encrypt data with AES-256-GCM using NanoTDF parameters
///
/// # Arguments
/// * `key` - AES-256 key
/// * `iv` - 3-byte IV (will be padded to 12 bytes)
/// * `plaintext` - Data to encrypt
/// * `tag_size` - Desired tag size (64-128 bits)
///
/// # Returns
/// Ciphertext || Tag (tag is appended to ciphertext)
pub fn encrypt(
    key: &AesKey,
    iv: &NanoTdfIv,
    plaintext: &[u8],
    tag_size: TagSize,
) -> Result<Vec<u8>, NanoTdfCryptoError> {
    let nonce_bytes = iv.to_gcm_nonce();

    match tag_size {
        TagSize::Bits96 => {
            let cipher = Aes256Gcm96::new(key.as_slice().into());
            cipher
                .encrypt((&nonce_bytes).into(), plaintext)
                .map_err(|_| NanoTdfCryptoError::EncryptionFailed)
        }
        TagSize::Bits104 => {
            let cipher = Aes256Gcm104::new(key.as_slice().into());
            cipher
                .encrypt((&nonce_bytes).into(), plaintext)
                .map_err(|_| NanoTdfCryptoError::EncryptionFailed)
        }
        TagSize::Bits112 => {
            let cipher = Aes256Gcm112::new(key.as_slice().into());
            cipher
                .encrypt((&nonce_bytes).into(), plaintext)
                .map_err(|_| NanoTdfCryptoError::EncryptionFailed)
        }
        TagSize::Bits120 => {
            let cipher = Aes256Gcm120::new(key.as_slice().into());
            cipher
                .encrypt((&nonce_bytes).into(), plaintext)
                .map_err(|_| NanoTdfCryptoError::EncryptionFailed)
        }
        TagSize::Bits128 => {
            let cipher = Aes256Gcm128::new(key.as_slice().into());
            cipher
                .encrypt((&nonce_bytes).into(), plaintext)
                .map_err(|_| NanoTdfCryptoError::EncryptionFailed)
        }
    }
}

/// Decrypt data with AES-256-GCM using NanoTDF parameters
///
/// # Arguments
/// * `key` - AES-256 key
/// * `iv` - 3-byte IV (will be padded to 12 bytes)
/// * `ciphertext_and_tag` - Ciphertext with appended tag
/// * `tag_size` - Expected tag size (64-128 bits)
///
/// # Returns
/// Decrypted plaintext
pub fn decrypt(
    key: &AesKey,
    iv: &NanoTdfIv,
    ciphertext_and_tag: &[u8],
    tag_size: TagSize,
) -> Result<Vec<u8>, NanoTdfCryptoError> {
    // Validate minimum length
    if ciphertext_and_tag.len() < tag_size.bytes() {
        return Err(NanoTdfCryptoError::InvalidTagLength {
            expected: tag_size.bytes(),
            actual: ciphertext_and_tag.len(),
        });
    }

    let nonce_bytes = iv.to_gcm_nonce();

    match tag_size {
        TagSize::Bits96 => {
            let cipher = Aes256Gcm96::new(key.as_slice().into());
            cipher
                .decrypt((&nonce_bytes).into(), ciphertext_and_tag)
                .map_err(|_| NanoTdfCryptoError::DecryptionFailed)
        }
        TagSize::Bits104 => {
            let cipher = Aes256Gcm104::new(key.as_slice().into());
            cipher
                .decrypt((&nonce_bytes).into(), ciphertext_and_tag)
                .map_err(|_| NanoTdfCryptoError::DecryptionFailed)
        }
        TagSize::Bits112 => {
            let cipher = Aes256Gcm112::new(key.as_slice().into());
            cipher
                .decrypt((&nonce_bytes).into(), ciphertext_and_tag)
                .map_err(|_| NanoTdfCryptoError::DecryptionFailed)
        }
        TagSize::Bits120 => {
            let cipher = Aes256Gcm120::new(key.as_slice().into());
            cipher
                .decrypt((&nonce_bytes).into(), ciphertext_and_tag)
                .map_err(|_| NanoTdfCryptoError::DecryptionFailed)
        }
        TagSize::Bits128 => {
            let cipher = Aes256Gcm128::new(key.as_slice().into());
            cipher
                .decrypt((&nonce_bytes).into(), ciphertext_and_tag)
                .map_err(|_| NanoTdfCryptoError::DecryptionFailed)
        }
    }
}

/// Generate GMAC tag for policy binding
///
/// GMAC is GCM with empty plaintext - the AAD becomes the message to authenticate.
/// Uses 96-bit (12-byte) tag as minimum supported by Rust aes-gcm crate.
pub fn generate_gmac(
    key: &AesKey,
    iv: &NanoTdfIv,
    data: &[u8],
) -> Result<[u8; 12], NanoTdfCryptoError> {
    // GMAC uses 96-bit tag (minimum supported)
    let cipher = Aes256Gcm96::new(key.as_slice().into());
    let nonce_bytes = iv.to_gcm_nonce();

    // GMAC: encrypt empty plaintext with AAD
    let payload = Payload {
        msg: &[],
        aad: data,
    };

    let result = cipher
        .encrypt((&nonce_bytes).into(), payload)
        .map_err(|_| NanoTdfCryptoError::EncryptionFailed)?;

    // Result is just the tag (12 bytes for GMAC)
    if result.len() != 12 {
        return Err(NanoTdfCryptoError::InvalidTagLength {
            expected: 12,
            actual: result.len(),
        });
    }

    let mut gmac = [0u8; 12];
    gmac.copy_from_slice(&result);
    Ok(gmac)
}

/// Verify GMAC tag for policy binding
pub fn verify_gmac(
    key: &AesKey,
    iv: &NanoTdfIv,
    data: &[u8],
    expected_tag: &[u8; 12],
) -> Result<bool, NanoTdfCryptoError> {
    let computed_tag = generate_gmac(key, iv, data)?;

    // Use constant-time comparison
    use subtle::ConstantTimeEq;
    Ok(computed_tag.ct_eq(expected_tag).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iv_conversion() {
        let iv = NanoTdfIv::from_bytes([0x01, 0x02, 0x03]);
        let nonce = iv.to_gcm_nonce();
        // Per NanoTDF spec: [9 zero bytes][3-byte IV]
        assert_eq!(&nonce[0..9], &[0u8; 9]); // First 9 bytes are zeros
        assert_eq!(&nonce[9..12], &[0x01, 0x02, 0x03]); // Last 3 bytes are the IV
    }

    #[test]
    fn test_encrypt_decrypt_all_tag_sizes() {
        let key = AesKey::from_slice(&[0x42u8; 32]).unwrap();
        let iv = NanoTdfIv::random();
        let plaintext = b"Test all NanoTDF tag sizes!";

        let tag_sizes = [
            TagSize::Bits96,
            TagSize::Bits104,
            TagSize::Bits112,
            TagSize::Bits120,
            TagSize::Bits128,
        ];

        for tag_size in tag_sizes {
            let ciphertext = encrypt(&key, &iv, plaintext, tag_size).unwrap();

            // Verify tag size is correct
            assert_eq!(
                ciphertext.len(),
                plaintext.len() + tag_size.bytes(),
                "Failed for {:?}",
                tag_size
            );

            // Decrypt and verify
            let decrypted = decrypt(&key, &iv, &ciphertext, tag_size).unwrap();
            assert_eq!(decrypted, plaintext, "Decryption failed for {:?}", tag_size);
        }
    }

    #[test]
    fn test_96bit_tag() {
        let key = AesKey::from_slice(&[0x42u8; 32]).unwrap();
        let iv = NanoTdfIv::random();
        let plaintext = b"Compact NanoTDF with 96-bit tag";

        let ciphertext = encrypt(&key, &iv, plaintext, TagSize::Bits96).unwrap();
        assert_eq!(ciphertext.len(), plaintext.len() + 12);

        let decrypted = decrypt(&key, &iv, &ciphertext, TagSize::Bits96).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_128bit_tag() {
        let key = AesKey::from_slice(&[0x42u8; 32]).unwrap();
        let iv = NanoTdfIv::random();
        let plaintext = b"Standard GCM with 128-bit tag";

        let ciphertext = encrypt(&key, &iv, plaintext, TagSize::Bits128).unwrap();
        assert_eq!(ciphertext.len(), plaintext.len() + 16);

        let decrypted = decrypt(&key, &iv, &ciphertext, TagSize::Bits128).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_gmac_generation() {
        let key = AesKey::from_slice(&[0x42u8; 32]).unwrap();
        let iv = NanoTdfIv::from_bytes([0x01, 0x02, 0x03]);
        let data = b"Policy data to authenticate";

        let gmac = generate_gmac(&key, &iv, data).unwrap();
        assert_eq!(gmac.len(), 12);

        // Verify returns true for correct tag
        assert!(verify_gmac(&key, &iv, data, &gmac).unwrap());

        // Verify returns false for wrong tag
        let wrong_gmac = [0u8; 12];
        assert!(!verify_gmac(&key, &iv, data, &wrong_gmac).unwrap());
    }

    #[test]
    fn test_wrong_tag_size_fails() {
        let key = AesKey::from_slice(&[0x42u8; 32]).unwrap();
        let iv = NanoTdfIv::random();
        let plaintext = b"Test";

        // Encrypt with 96-bit tag
        let ciphertext = encrypt(&key, &iv, plaintext, TagSize::Bits96).unwrap();

        // Try to decrypt with wrong tag size (should fail)
        let result = decrypt(&key, &iv, &ciphertext, TagSize::Bits128);
        assert!(result.is_err());
    }

    #[test]
    fn test_policy_iv_reserved() {
        let iv = NanoTdfIv::POLICY_IV;
        assert_eq!(iv.as_bytes(), &[0x00, 0x00, 0x00]);
    }
}
