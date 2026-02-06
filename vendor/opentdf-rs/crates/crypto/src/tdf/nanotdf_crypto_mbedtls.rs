//! NanoTDF Cryptographic Operations using Mbed TLS
//!
//! This backend provides full GCM tag size support (64-128 bits) using Mbed TLS.
//! This is the recommended backend for NanoTDF as 64-bit tags are the spec default.

use crate::types::AesKey;
use mbedtls::cipher::{Cipher, Decryption, Encryption, Fresh};
use rand::RngCore;
use thiserror::Error;

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

    #[error("Mbed TLS error: {0}")]
    MbedTlsError(String),
}

impl From<mbedtls::Error> for NanoTdfCryptoError {
    fn from(err: mbedtls::Error) -> Self {
        NanoTdfCryptoError::MbedTlsError(format!("{:?}", err))
    }
}

/// Tag size for AES-256-GCM
///
/// Mbed TLS backend supports full 64-128 bit tag range
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagSize {
    /// 64-bit tag (8 bytes) - NanoTDF default
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

    /// Convert to 12-byte nonce for AES-GCM (pad with zeros)
    pub fn to_gcm_nonce(&self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[0..3].copy_from_slice(&self.0);
        nonce
    }

    /// Reserved IV for encrypted policy (must not be reused)
    pub const POLICY_IV: Self = Self([0x00, 0x00, 0x00]);
}

/// Encrypt data with AES-256-GCM using NanoTDF parameters (Mbed TLS backend)
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
    use mbedtls::cipher::raw::{CipherId, CipherMode};

    // Create AES-256-GCM cipher in encryption mode
    let mut cipher = Cipher::<Encryption, _, _>::new(CipherId::Aes, CipherMode::GCM, 256)?;

    // Set key and IV
    cipher.set_key_iv(Encryption, key.as_slice(), Some(&iv.to_gcm_nonce()))?;

    // Prepare output buffer
    let mut ciphertext = vec![0u8; plaintext.len() + tag_size.bytes()];

    // Encrypt with GCM
    let (len, tag) = cipher.encrypt_auth(
        &[], // No AAD
        plaintext,
        &mut ciphertext[..plaintext.len()],
        tag_size.bytes(),
    )?;

    // Append tag
    ciphertext[len..len + tag.len()].copy_from_slice(&tag);
    ciphertext.truncate(len + tag.len());

    Ok(ciphertext)
}

/// Decrypt data with AES-256-GCM using NanoTDF parameters (Mbed TLS backend)
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

    use mbedtls::cipher::raw::{CipherId, CipherMode};

    // Split ciphertext and tag
    let ciphertext_len = ciphertext_and_tag.len() - tag_size.bytes();
    let ciphertext = &ciphertext_and_tag[..ciphertext_len];
    let tag = &ciphertext_and_tag[ciphertext_len..];

    // Create AES-256-GCM cipher in decryption mode
    let mut cipher = Cipher::<Decryption, _, _>::new(CipherId::Aes, CipherMode::GCM, 256)?;

    // Set key and IV
    cipher.set_key_iv(Decryption, key.as_slice(), Some(&iv.to_gcm_nonce()))?;

    // Prepare output buffer
    let mut plaintext = vec![0u8; ciphertext_len];

    // Decrypt with GCM
    let len = cipher.decrypt_auth(
        &[], // No AAD
        ciphertext,
        &mut plaintext,
        tag,
    )?;

    plaintext.truncate(len);
    Ok(plaintext)
}

/// Generate GMAC tag for policy binding (Mbed TLS backend)
///
/// GMAC is GCM with empty plaintext - the AAD becomes the message to authenticate.
/// Uses 64-bit (8-byte) tag as NanoTDF default.
pub fn generate_gmac(
    key: &AesKey,
    iv: &NanoTdfIv,
    data: &[u8],
) -> Result<[u8; 8], NanoTdfCryptoError> {
    use mbedtls::cipher::raw::{CipherId, CipherMode};

    // GMAC uses 64-bit tag (NanoTDF default)
    let mut cipher = Cipher::<Encryption, _, _>::new(CipherId::Aes, CipherMode::GCM, 256)?;
    cipher.set_key_iv(Encryption, key.as_slice(), Some(&iv.to_gcm_nonce()))?;

    // GMAC: encrypt empty plaintext with AAD
    let mut output = vec![];
    let (_, tag) = cipher.encrypt_auth(
        data, // AAD is the data to authenticate
        &[],  // Empty plaintext
        &mut output,
        8, // 64-bit tag
    )?;

    let mut gmac = [0u8; 8];
    gmac.copy_from_slice(&tag[..8]);
    Ok(gmac)
}

/// Verify GMAC tag for policy binding (Mbed TLS backend)
pub fn verify_gmac(
    key: &AesKey,
    iv: &NanoTdfIv,
    data: &[u8],
    expected_tag: &[u8; 8],
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
        assert_eq!(&nonce[0..3], &[0x01, 0x02, 0x03]);
        assert_eq!(&nonce[3..12], &[0u8; 9]); // Rest is zeros
    }

    #[test]
    fn test_encrypt_decrypt_all_tag_sizes() {
        let key = AesKey::from_slice(&[0x42u8; 32]).unwrap();
        let iv = NanoTdfIv::random();
        let plaintext = b"Test all NanoTDF tag sizes!";

        let tag_sizes = [
            TagSize::Bits64,
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
    fn test_64bit_tag() {
        let key = AesKey::from_slice(&[0x42u8; 32]).unwrap();
        let iv = NanoTdfIv::random();
        let plaintext = b"Compact NanoTDF with 64-bit tag";

        let ciphertext = encrypt(&key, &iv, plaintext, TagSize::Bits64).unwrap();
        assert_eq!(ciphertext.len(), plaintext.len() + 8);

        let decrypted = decrypt(&key, &iv, &ciphertext, TagSize::Bits64).unwrap();
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
        assert_eq!(gmac.len(), 8);

        // Verify returns true for correct tag
        assert!(verify_gmac(&key, &iv, data, &gmac).unwrap());

        // Verify returns false for wrong tag
        let wrong_gmac = [0u8; 8];
        assert!(!verify_gmac(&key, &iv, data, &wrong_gmac).unwrap());
    }

    #[test]
    fn test_wrong_tag_size_fails() {
        let key = AesKey::from_slice(&[0x42u8; 32]).unwrap();
        let iv = NanoTdfIv::random();
        let plaintext = b"Test";

        // Encrypt with 64-bit tag
        let ciphertext = encrypt(&key, &iv, plaintext, TagSize::Bits64).unwrap();

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
