//! TDF Encryption implementation with zeroizing keys
//!
//! This module implements segment-based encryption for OpenTDF standard TDF format.
//! All cryptographic key material uses zeroizing types for secure memory handling.

// Allow deprecated warnings for Nonce::from_slice() which is the correct API for aes-gcm 0.10.x
#![allow(deprecated)]

use crate::helpers::{generate_key_32, generate_nonce};
use crate::types::{PayloadKey, PolicyKey};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EncryptionError {
    #[error("Encryption failed: {0}")]
    AeadError(aes_gcm::Error),

    #[error("Key generation failed")]
    KeyGenerationError,

    #[error("Invalid key length")]
    InvalidKeyLength,

    #[error("Base64 decode error: {0}")]
    Base64Error(#[from] base64::DecodeError),

    #[error("Segment error: {0}")]
    SegmentError(String),
}

/// Encrypted payload structure
#[derive(Debug, Serialize, Deserialize)]
pub struct EncryptedPayload {
    /// Base64 encoded encrypted data
    pub ciphertext: String,

    /// Base64 encoded initialization vector
    pub iv: String,

    /// Base64 encoded encrypted payload key
    pub encrypted_key: String,

    /// Hash of the policy key for verification
    pub policy_key_hash: String,
}

/// TDF encryption engine with zeroizing keys
pub struct TdfEncryption {
    policy_key: PolicyKey,
    payload_key: PayloadKey,
}

impl TdfEncryption {
    /// Create a new TdfEncryption instance with generated keys
    pub fn new() -> Result<Self, EncryptionError> {
        let policy_key_bytes = generate_key_32();
        let payload_key_bytes = generate_key_32();

        Ok(Self {
            policy_key: PolicyKey::from_slice(&policy_key_bytes)
                .map_err(|_| EncryptionError::InvalidKeyLength)?,
            payload_key: PayloadKey::from_slice(&payload_key_bytes)
                .map_err(|_| EncryptionError::InvalidKeyLength)?,
        })
    }

    /// Create a TdfEncryption instance with an existing policy key
    pub fn with_policy_key(policy_key: &[u8]) -> Result<Self, EncryptionError> {
        let payload_key_bytes = generate_key_32();

        Ok(Self {
            policy_key: PolicyKey::from_slice(policy_key)
                .map_err(|_| EncryptionError::InvalidKeyLength)?,
            payload_key: PayloadKey::from_slice(&payload_key_bytes)
                .map_err(|_| EncryptionError::InvalidKeyLength)?,
        })
    }

    /// Create TdfEncryption with a known payload key (for KAS decryption)
    ///
    /// When decrypting a TDF with KAS, the unwrapped key from KAS IS the payload key.
    /// This constructor uses that key directly without generating a new random one.
    pub fn with_payload_key(payload_key: &[u8]) -> Result<Self, EncryptionError> {
        let policy_key_bytes = [0u8; 32]; // Not used for decryption

        Ok(Self {
            policy_key: PolicyKey::from_slice(&policy_key_bytes)
                .map_err(|_| EncryptionError::InvalidKeyLength)?,
            payload_key: PayloadKey::from_slice(payload_key)
                .map_err(|_| EncryptionError::InvalidKeyLength)?,
        })
    }

    /// Encrypt data using the payload key and then encrypt the payload key using the policy key
    pub fn encrypt(&self, data: &[u8]) -> Result<EncryptedPayload, EncryptionError> {
        // Generate random IV for payload encryption
        let payload_iv = generate_nonce();

        // Encrypt the actual data using the payload key
        let payload_cipher = Aes256Gcm::new_from_slice(self.payload_key.as_slice())
            .map_err(|_| EncryptionError::InvalidKeyLength)?;
        let nonce = Nonce::from_slice(payload_iv.as_slice());
        let ciphertext = payload_cipher
            .encrypt(nonce, data)
            .map_err(EncryptionError::AeadError)?;

        // Generate random IV for key encryption
        let key_iv = generate_nonce();

        // Encrypt the payload key using the policy key
        let policy_cipher = Aes256Gcm::new_from_slice(self.policy_key.as_slice())
            .map_err(|_| EncryptionError::InvalidKeyLength)?;
        let key_nonce = Nonce::from_slice(key_iv.as_slice());
        let encrypted_key = policy_cipher
            .encrypt(key_nonce, self.payload_key.as_slice())
            .map_err(EncryptionError::AeadError)?;

        // Calculate policy key hash for verification
        let mut hasher = Sha256::new();
        hasher.update(self.policy_key.as_slice());
        let policy_key_hash = hasher.finalize();

        Ok(EncryptedPayload {
            ciphertext: BASE64.encode(ciphertext),
            iv: BASE64.encode([payload_iv.as_slice(), key_iv.as_slice()].concat()),
            encrypted_key: BASE64.encode(encrypted_key),
            policy_key_hash: BASE64.encode(policy_key_hash),
        })
    }

    /// Decrypt data using the policy key to first decrypt the payload key
    pub fn decrypt(
        &mut self,
        ciphertext: &[u8],
        iv: &[u8],
        encrypted_key: &[u8],
    ) -> Result<Vec<u8>, EncryptionError> {
        // First decrypt the payload key using the policy key
        let policy_cipher = Aes256Gcm::new_from_slice(self.policy_key.as_slice())
            .map_err(|_| EncryptionError::InvalidKeyLength)?;
        let nonce = Nonce::from_slice(iv);

        let decrypted_payload_key = policy_cipher
            .decrypt(nonce, encrypted_key)
            .map_err(EncryptionError::AeadError)?;

        // Update payload key
        self.payload_key = PayloadKey::from_slice(&decrypted_payload_key)
            .map_err(|_| EncryptionError::InvalidKeyLength)?;

        // Then decrypt the actual data using the decrypted payload key
        let payload_cipher = Aes256Gcm::new_from_slice(self.payload_key.as_slice())
            .map_err(|_| EncryptionError::InvalidKeyLength)?;
        let payload_nonce = Nonce::from_slice(iv);
        let plaintext = payload_cipher
            .decrypt(payload_nonce, ciphertext)
            .map_err(EncryptionError::AeadError)?;

        Ok(plaintext)
    }

    /// Decrypt using the old format with combined IVs (for backward compatibility)
    pub fn decrypt_legacy(
        policy_key: &[u8],
        encrypted_payload: &EncryptedPayload,
    ) -> Result<Vec<u8>, EncryptionError> {
        // Decode base64 values
        let ciphertext = BASE64.decode(&encrypted_payload.ciphertext)?;
        let combined_iv = BASE64.decode(&encrypted_payload.iv)?;
        let encrypted_key = BASE64.decode(&encrypted_payload.encrypted_key)?;

        // Split combined IV into payload and key IVs
        let (payload_iv, key_iv) = combined_iv.split_at(12);

        // First decrypt the payload key using the policy key
        let policy_cipher =
            Aes256Gcm::new_from_slice(policy_key).map_err(|_| EncryptionError::InvalidKeyLength)?;
        let key_nonce = Nonce::from_slice(key_iv);
        let payload_key = policy_cipher
            .decrypt(key_nonce, encrypted_key.as_ref())
            .map_err(EncryptionError::AeadError)?;

        // Then decrypt the actual data using the decrypted payload key
        let payload_cipher = Aes256Gcm::new_from_slice(&payload_key)
            .map_err(|_| EncryptionError::InvalidKeyLength)?;
        let payload_nonce = Nonce::from_slice(payload_iv);
        let plaintext = payload_cipher
            .decrypt(payload_nonce, ciphertext.as_ref())
            .map_err(EncryptionError::AeadError)?;

        Ok(plaintext)
    }

    /// Get the policy key
    pub fn policy_key(&self) -> &[u8] {
        self.policy_key.as_slice()
    }

    /// Get the payload key
    pub fn payload_key(&self) -> &[u8] {
        self.payload_key.as_slice()
    }

    /// Encrypt data using segment-based encryption for OpenTDF compatibility
    ///
    /// This implements the OpenTDF standard segment-based encryption:
    /// - Splits payload into segments (default 2MB)
    /// - Encrypts each segment with AES-256-GCM
    /// - Extracts GMAC tag (last 16 bytes) from each encrypted segment
    /// - Returns segment data and metadata for manifest generation
    pub fn encrypt_with_segments(
        &self,
        data: &[u8],
        segment_size: usize,
    ) -> Result<SegmentedPayload, EncryptionError> {
        const GCM_TAG_SIZE: usize = 16; // 128-bit authentication tag

        let cipher = Aes256Gcm::new_from_slice(self.payload_key.as_slice())
            .map_err(|_| EncryptionError::InvalidKeyLength)?;

        // Pre-allocate vectors with expected capacity for performance
        let segment_count = data.len().div_ceil(segment_size);
        let mut segments = Vec::with_capacity(segment_count);
        let mut segment_info = Vec::with_capacity(segment_count);
        let mut gmac_tags = Vec::with_capacity(segment_count);

        // Process data in segments
        for chunk in data.chunks(segment_size) {
            // Generate unique IV for this segment
            let iv = generate_nonce();
            let nonce = Nonce::from_slice(iv.as_slice());

            // Encrypt segment
            let ciphertext = cipher
                .encrypt(nonce, chunk)
                .map_err(EncryptionError::AeadError)?;

            // AES-GCM output format: [encrypted_data][16-byte auth tag]
            // GMAC is the authentication tag (last 16 bytes)
            let gmac_tag = ciphertext[ciphertext.len() - GCM_TAG_SIZE..].to_vec();

            // Prepend IV to ciphertext for storage (OpenTDF format)
            let mut segment_data = iv.as_slice().to_vec();
            segment_data.extend_from_slice(&ciphertext);

            segment_info.push(SegmentInfo {
                hash: BASE64.encode(&gmac_tag),
                plaintext_size: chunk.len() as u64,
                encrypted_size: segment_data.len() as u64,
            });

            segments.push(segment_data);
            gmac_tags.push(gmac_tag);
        }

        Ok(SegmentedPayload {
            segments,
            segment_info,
            gmac_tags,
        })
    }

    /// Decrypt data using segment-based decryption for OpenTDF compatibility
    ///
    /// This implements the OpenTDF standard segment-based decryption:
    /// - Parses payload into segments based on segment metadata
    /// - Each segment format: [IV (12 bytes)][Ciphertext + Tag]
    /// - Decrypts each segment with AES-256-GCM
    /// - Extracts GMAC tag (last 16 bytes) from each encrypted segment
    /// - Returns plaintext and GMAC tags for root signature verification
    ///
    /// # Arguments
    ///
    /// * `payload` - The encrypted payload bytes (concatenated segments)
    /// * `segments` - Segment metadata (encrypted sizes)
    ///
    /// # Returns
    ///
    /// Tuple of (plaintext, gmac_tags) for verification.
    ///
    /// # Security Note
    ///
    /// The returned plaintext is the caller's responsibility. Consider wrapping
    /// in `zeroize::Zeroizing` if the content is sensitive and should be wiped
    /// from memory when no longer needed.
    pub fn decrypt_with_segments(
        &self,
        payload: &[u8],
        segments: &[(u64, u64)], // (plaintext_size, encrypted_size)
    ) -> Result<(Vec<u8>, Vec<Vec<u8>>), EncryptionError> {
        const GCM_IV_SIZE: usize = 12; // 96-bit IV
        const GCM_TAG_SIZE: usize = 16; // 128-bit authentication tag

        let cipher = Aes256Gcm::new_from_slice(self.payload_key.as_slice())
            .map_err(|_| EncryptionError::InvalidKeyLength)?;

        // Pre-allocate with expected capacity
        let total_plaintext_size: usize = segments.iter().map(|(pt, _)| *pt as usize).sum();
        let mut plaintext = Vec::with_capacity(total_plaintext_size);
        let mut gmac_tags = Vec::with_capacity(segments.len());
        let mut offset = 0;

        for (_plaintext_size, encrypted_size) in segments {
            let encrypted_size = *encrypted_size as usize;

            // Ensure we don't read past the payload
            if offset + encrypted_size > payload.len() {
                return Err(EncryptionError::SegmentError(
                    "Segment extends beyond payload".to_string(),
                ));
            }

            // Extract segment data
            let segment_data = &payload[offset..offset + encrypted_size];

            // Parse segment: [IV (12)][Ciphertext + Tag]
            if segment_data.len() < GCM_IV_SIZE + GCM_TAG_SIZE {
                return Err(EncryptionError::SegmentError(
                    "Segment too small".to_string(),
                ));
            }

            let iv = &segment_data[..GCM_IV_SIZE];
            let ciphertext_and_tag = &segment_data[GCM_IV_SIZE..];

            // Decrypt segment
            let nonce = Nonce::from_slice(iv);
            let decrypted = cipher
                .decrypt(nonce, ciphertext_and_tag)
                .map_err(EncryptionError::AeadError)?;

            // Extract GMAC tag (last 16 bytes of ciphertext_and_tag before decryption)
            let gmac_tag = ciphertext_and_tag[ciphertext_and_tag.len() - GCM_TAG_SIZE..].to_vec();
            gmac_tags.push(gmac_tag);

            plaintext.extend_from_slice(&decrypted);
            offset += encrypted_size;
        }

        Ok((plaintext, gmac_tags))
    }
}

impl Default for TdfEncryption {
    fn default() -> Self {
        Self::new().expect("Failed to generate keys")
    }
}

/// Information about an encrypted segment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentInfo {
    /// Base64 encoded GMAC tag
    pub hash: String,

    /// Size before encryption
    pub plaintext_size: u64,

    /// Size after encryption (includes IV + tag)
    pub encrypted_size: u64,
}

/// Result of segment-based encryption
#[derive(Debug)]
pub struct SegmentedPayload {
    /// Encrypted segment data (IV + ciphertext + tag)
    pub segments: Vec<Vec<u8>>,

    /// Metadata for manifest
    pub segment_info: Vec<SegmentInfo>,

    /// Raw GMAC tags for root signature
    pub gmac_tags: Vec<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() -> Result<(), EncryptionError> {
        let tdf = TdfEncryption::new()?;
        let data = b"Hello, TDF!";

        let encrypted = tdf.encrypt(data)?;
        let decrypted = TdfEncryption::decrypt_legacy(tdf.policy_key(), &encrypted)?;

        assert_eq!(data, decrypted.as_slice());
        Ok(())
    }

    #[test]
    fn test_segment_encryption_and_decryption() -> Result<(), EncryptionError> {
        let tdf = TdfEncryption::new()?;
        let plaintext =
            b"Hello from segment test! This is a test of segment-based encryption and decryption.";

        const SEGMENT_SIZE: usize = 32;
        let segmented = tdf.encrypt_with_segments(plaintext, SEGMENT_SIZE)?;

        let expected_segments = plaintext.len().div_ceil(SEGMENT_SIZE);
        assert_eq!(segmented.segments.len(), expected_segments);

        // Create segment metadata for decryption
        let segments: Vec<(u64, u64)> = segmented
            .segment_info
            .iter()
            .map(|info| (info.plaintext_size, info.encrypted_size))
            .collect();

        // Concatenate all segment data
        let mut payload = Vec::new();
        for segment in &segmented.segments {
            payload.extend_from_slice(segment);
        }

        // Decrypt
        let (decrypted, gmac_tags) = tdf.decrypt_with_segments(&payload, &segments)?;

        assert_eq!(plaintext, decrypted.as_slice());
        assert_eq!(gmac_tags.len(), segmented.gmac_tags.len());

        Ok(())
    }
}
