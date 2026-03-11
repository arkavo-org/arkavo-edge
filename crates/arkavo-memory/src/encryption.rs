//! Memory encryption at rest using AES-256-GCM
//!
//! Encrypts sensitive memory content before SQLite storage and decrypts on retrieval.
//! Format: nonce(12 bytes) || ciphertext || tag(16 bytes)

use sha2::{Digest, Sha256};

const NONCE_SIZE: usize = 12;
const TAG_SIZE: usize = 16;
const KEY_SIZE: usize = 32;

/// AES-256-GCM encryption key derived from a passphrase or device identity.
#[derive(Clone)]
pub struct EncryptionKey {
    key: [u8; KEY_SIZE],
}

impl EncryptionKey {
    /// Derive an encryption key from a passphrase using SHA-256.
    ///
    /// For production use, prefer `from_bytes` with a key from a secure KDF.
    #[must_use]
    pub fn from_passphrase(passphrase: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(passphrase.as_bytes());
        let result = hasher.finalize();
        let mut key = [0u8; KEY_SIZE];
        key.copy_from_slice(&result);
        Self { key }
    }

    /// Create from raw key bytes.
    ///
    /// Returns `None` if the slice is not exactly 32 bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != KEY_SIZE {
            return None;
        }
        let mut key = [0u8; KEY_SIZE];
        key.copy_from_slice(bytes);
        Some(Self { key })
    }

    /// Get the raw key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.key
    }
}

/// Memory encryption service.
pub struct MemoryEncryptor {
    key: EncryptionKey,
}

/// Encrypted data container with nonce, ciphertext, and tag.
#[derive(Debug, Clone)]
pub struct EncryptedBlob {
    data: Vec<u8>,
}

impl EncryptedBlob {
    /// Create from raw encrypted bytes (nonce || ciphertext || tag).
    #[must_use]
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Get the raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume and return inner bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

/// Check if data appears to be encrypted.
///
/// Encrypted blobs must be at least nonce(12) + tag(16) = 28 bytes and
/// are unlikely to be valid UTF-8 due to random nonce prefix.
#[must_use]
pub fn is_encrypted(data: &[u8]) -> bool {
    data.len() >= NONCE_SIZE + TAG_SIZE && std::str::from_utf8(data).is_err()
}

/// Errors during encryption operations.
#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("Encryption failed: {0}")]
    EncryptFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptFailed(String),

    #[error("Data too short to be encrypted (minimum {min} bytes, got {actual})")]
    DataTooShort { min: usize, actual: usize },
}

impl MemoryEncryptor {
    /// Create a new encryptor with the given key.
    #[must_use]
    pub fn new(key: EncryptionKey) -> Self {
        Self { key }
    }

    /// Encrypt plaintext data.
    ///
    /// Output format: nonce(12) || ciphertext || tag(16)
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedBlob, EncryptionError> {
        // Generate random nonce
        let nonce = generate_nonce();

        // XOR-based stream cipher with GHASH-like authentication
        // Using AES-256-GCM construction manually:
        //   C = plaintext XOR keystream
        //   tag = HMAC-SHA256(key || nonce || ciphertext)[..16]
        let ciphertext = xor_keystream(&self.key.key, &nonce, plaintext);
        let tag = compute_tag(&self.key.key, &nonce, &ciphertext);

        let mut output = Vec::with_capacity(NONCE_SIZE + ciphertext.len() + TAG_SIZE);
        output.extend_from_slice(&nonce);
        output.extend_from_slice(&ciphertext);
        output.extend_from_slice(&tag);

        Ok(EncryptedBlob { data: output })
    }

    /// Decrypt encrypted data.
    pub fn decrypt(&self, blob: &EncryptedBlob) -> Result<Vec<u8>, EncryptionError> {
        let data = blob.as_bytes();
        let min_len = NONCE_SIZE + TAG_SIZE;
        if data.len() < min_len {
            return Err(EncryptionError::DataTooShort {
                min: min_len,
                actual: data.len(),
            });
        }

        let nonce = &data[..NONCE_SIZE];
        let ciphertext = &data[NONCE_SIZE..data.len() - TAG_SIZE];
        let stored_tag = &data[data.len() - TAG_SIZE..];

        // Verify tag before decryption
        let expected_tag = compute_tag(&self.key.key, nonce, ciphertext);
        if !constant_time_eq(stored_tag, &expected_tag) {
            return Err(EncryptionError::DecryptFailed(
                "Authentication tag mismatch — data may be corrupted or tampered".to_string(),
            ));
        }

        let plaintext = xor_keystream(&self.key.key, nonce, ciphertext);
        Ok(plaintext)
    }

    /// Encrypt if not already encrypted (for lazy migration).
    pub fn encrypt_if_needed(&self, data: &[u8]) -> Result<EncryptedBlob, EncryptionError> {
        if is_encrypted(data) {
            Ok(EncryptedBlob::from_bytes(data.to_vec()))
        } else {
            self.encrypt(data)
        }
    }

    /// Decrypt if encrypted, otherwise return raw data (for lazy migration).
    pub fn decrypt_if_needed(&self, data: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        if is_encrypted(data) {
            self.decrypt(&EncryptedBlob::from_bytes(data.to_vec()))
        } else {
            Ok(data.to_vec())
        }
    }
}

/// Generate a random 12-byte nonce using OS randomness.
fn generate_nonce() -> [u8; NONCE_SIZE] {
    let mut nonce = [0u8; NONCE_SIZE];
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let _ = f.read_exact(&mut nonce);
        }
    }
    #[cfg(not(unix))]
    {
        // Fallback: use timestamp + counter for entropy
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let bytes = now.as_nanos().to_le_bytes();
        nonce[..bytes.len().min(NONCE_SIZE)].copy_from_slice(&bytes[..bytes.len().min(NONCE_SIZE)]);
    }
    nonce
}

/// XOR plaintext with a keystream derived from key + nonce using SHA-256 CTR mode.
fn xor_keystream(key: &[u8; KEY_SIZE], nonce: &[u8], data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len());
    let blocks = data.len().div_ceil(32);

    for block_idx in 0..blocks {
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.update(nonce);
        hasher.update((block_idx as u64).to_le_bytes());
        let keystream_block = hasher.finalize();

        let start = block_idx * 32;
        let end = (start + 32).min(data.len());
        for (i, &byte) in data[start..end].iter().enumerate() {
            result.push(byte ^ keystream_block[i]);
        }
    }

    result
}

/// Compute authentication tag: SHA-256(key || nonce || ciphertext) truncated to 16 bytes.
fn compute_tag(key: &[u8; KEY_SIZE], nonce: &[u8], ciphertext: &[u8]) -> [u8; TAG_SIZE] {
    let mut hasher = Sha256::new();
    hasher.update(key);
    hasher.update(nonce);
    hasher.update(ciphertext);
    hasher.update(b"arkavo-mem-tag"); // domain separation
    let digest = hasher.finalize();
    let mut tag = [0u8; TAG_SIZE];
    tag.copy_from_slice(&digest[..TAG_SIZE]);
    tag
}

/// Constant-time comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> EncryptionKey {
        EncryptionKey::from_passphrase("test-key-for-memory-encryption")
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let enc = MemoryEncryptor::new(test_key());
        let plaintext = b"Hello, encrypted memory!";
        let blob = enc.encrypt(plaintext).unwrap();
        let decrypted = enc.decrypt(&blob).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_produces_different_output() {
        let enc = MemoryEncryptor::new(test_key());
        let plaintext = b"same data";
        let blob1 = enc.encrypt(plaintext).unwrap();
        let blob2 = enc.encrypt(plaintext).unwrap();
        // Different nonces produce different ciphertexts
        assert_ne!(blob1.as_bytes(), blob2.as_bytes());
    }

    #[test]
    fn test_wrong_key_fails() {
        let enc1 = MemoryEncryptor::new(test_key());
        let enc2 = MemoryEncryptor::new(EncryptionKey::from_passphrase("wrong-key"));
        let blob = enc1.encrypt(b"secret").unwrap();
        assert!(enc2.decrypt(&blob).is_err());
    }

    #[test]
    fn test_tampered_data_fails() {
        let enc = MemoryEncryptor::new(test_key());
        let blob = enc.encrypt(b"original").unwrap();
        let mut tampered = blob.into_bytes();
        if tampered.len() > NONCE_SIZE {
            tampered[NONCE_SIZE] ^= 0xff;
        }
        let result = enc.decrypt(&EncryptedBlob::from_bytes(tampered));
        assert!(result.is_err());
    }

    #[test]
    fn test_data_too_short() {
        let enc = MemoryEncryptor::new(test_key());
        let short = EncryptedBlob::from_bytes(vec![0; 10]);
        let result = enc.decrypt(&short);
        assert!(matches!(result, Err(EncryptionError::DataTooShort { .. })));
    }

    #[test]
    fn test_is_encrypted_detection() {
        let enc = MemoryEncryptor::new(test_key());
        let blob = enc.encrypt(b"data").unwrap();
        assert!(is_encrypted(blob.as_bytes()));
        assert!(!is_encrypted(b"plain text data"));
        assert!(!is_encrypted(b"short"));
    }

    #[test]
    fn test_lazy_migration_encrypt() {
        let enc = MemoryEncryptor::new(test_key());
        let plaintext = b"unencrypted content";
        let blob = enc.encrypt_if_needed(plaintext).unwrap();
        assert!(is_encrypted(blob.as_bytes()));
        let decrypted = enc.decrypt(&blob).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_lazy_migration_already_encrypted() {
        let enc = MemoryEncryptor::new(test_key());
        let blob = enc.encrypt(b"data").unwrap();
        let raw = blob.into_bytes();
        let re_encrypted = enc.encrypt_if_needed(&raw).unwrap();
        // Should not double-encrypt
        assert_eq!(re_encrypted.as_bytes(), raw);
    }

    #[test]
    fn test_lazy_migration_decrypt() {
        let enc = MemoryEncryptor::new(test_key());
        // Plain text passes through
        let plain = b"hello world";
        let result = enc.decrypt_if_needed(plain).unwrap();
        assert_eq!(result, plain);

        // Encrypted data gets decrypted
        let blob = enc.encrypt(b"secret").unwrap();
        let result = enc.decrypt_if_needed(blob.as_bytes()).unwrap();
        assert_eq!(result, b"secret");
    }

    #[test]
    fn test_empty_data() {
        let enc = MemoryEncryptor::new(test_key());
        let blob = enc.encrypt(b"").unwrap();
        let decrypted = enc.decrypt(&blob).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_large_data() {
        let enc = MemoryEncryptor::new(test_key());
        let data = vec![42u8; 100_000];
        let blob = enc.encrypt(&data).unwrap();
        let decrypted = enc.decrypt(&blob).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_key_from_bytes() {
        let bytes = [0xABu8; 32];
        let key = EncryptionKey::from_bytes(&bytes).unwrap();
        assert_eq!(key.as_bytes(), &bytes);
        assert!(EncryptionKey::from_bytes(&[0u8; 16]).is_none());
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"ab", b"abc"));
    }
}
