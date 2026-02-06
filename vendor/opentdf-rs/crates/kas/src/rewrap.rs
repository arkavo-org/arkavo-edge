//! Rewrap operations for KAS server
//!
//! Provides HKDF key derivation and AES-GCM encryption for rewrapping DEKs.

use crate::error::KasServerError;
use aes_gcm::Aes256Gcm;
use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::{Aead, Key, KeyInit};
use hkdf::Hkdf;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::Sha256;

/// NanoTDF-compatible rewrap operation
///
/// Derives the actual DEK from ECDH shared secret, then wraps it for transport.
///
/// # Steps
/// 1. Derive DEK from dek_shared_secret using HKDF with salt
/// 2. Derive wrapping key from session_shared_secret using HKDF with salt
/// 3. Encrypt derived DEK with AES-256-GCM using wrapping key
pub fn rewrap_dek(
    dek_shared_secret: &[u8],
    session_shared_secret: &[u8],
    salt: &[u8],
    info: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), KasServerError> {
    // Derive the actual DEK from the TDF ECDH shared secret
    let dek_hkdf = Hkdf::<Sha256>::new(Some(salt), dek_shared_secret);
    let mut dek = [0u8; 32];
    dek_hkdf.expand(info, &mut dek)?;

    // Derive the wrapping key from the session shared secret
    let session_hkdf = Hkdf::<Sha256>::new(Some(salt), session_shared_secret);
    let mut wrapping_key = [0u8; 32];
    session_hkdf.expand(info, &mut wrapping_key)?;

    // Generate random nonce (12 bytes for AES-GCM)
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let nonce_ga = GenericArray::from_slice(&nonce);

    // Wrap the derived DEK with AES-256-GCM
    let key = Key::<Aes256Gcm>::from(wrapping_key);
    let cipher = Aes256Gcm::new(&key);
    let wrapped_dek = cipher.encrypt(nonce_ga, &dek[..])?;

    Ok((nonce.to_vec(), wrapped_dek))
}

/// Simple rewrap for RSA-unwrapped DEKs
///
/// Encrypts the DEK directly with session shared secret using HKDF + AES-GCM.
pub fn rewrap_dek_simple(
    dek: &[u8],
    session_shared_secret: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), KasServerError> {
    // Derive symmetric key using HKDF (no salt, empty info)
    let hkdf = Hkdf::<Sha256>::new(None, session_shared_secret);
    let mut derived_key = [0u8; 32];
    hkdf.expand(b"", &mut derived_key)?;

    // Generate random nonce (12 bytes for AES-GCM)
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let nonce_ga = GenericArray::from_slice(&nonce);

    // Encrypt DEK with AES-256-GCM
    let key = Key::<Aes256Gcm>::from(derived_key);
    let cipher = Aes256Gcm::new(&key);
    let wrapped_dek = cipher.encrypt(nonce_ga, dek)?;

    Ok((nonce.to_vec(), wrapped_dek))
}

/// Combine nonce and wrapped DEK into a single base64-encoded string
pub fn encode_wrapped_key(nonce: &[u8], wrapped_dek: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut combined = Vec::with_capacity(nonce.len() + wrapped_dek.len());
    combined.extend_from_slice(nonce);
    combined.extend_from_slice(wrapped_dek);
    STANDARD.encode(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::salt::{NanoTdfVersion, compute_nanotdf_salt};

    #[test]
    fn test_rewrap_dek() {
        let dek_secret = b"test_dek_shared_secret__32bytes!";
        let session_secret = b"test_session_shared_secret__32b!";
        let salt = compute_nanotdf_salt(NanoTdfVersion::V12);

        let result = rewrap_dek(dek_secret, session_secret, &salt, b"");
        assert!(result.is_ok());

        let (nonce, wrapped) = result.unwrap();
        assert_eq!(nonce.len(), 12);
        // Wrapped should be 32-byte DEK + 16-byte tag = 48 bytes
        assert_eq!(wrapped.len(), 48);
    }

    #[test]
    fn test_rewrap_dek_simple() {
        let dek = b"test_data_encryption_key_32bytes";
        let session_secret = b"test_session_shared_secret__32b!";

        let result = rewrap_dek_simple(dek, session_secret);
        assert!(result.is_ok());

        let (nonce, wrapped) = result.unwrap();
        assert_eq!(nonce.len(), 12);
        // Wrapped should be ciphertext + 16-byte auth tag
        assert_eq!(wrapped.len(), dek.len() + 16);
    }

    #[test]
    fn test_encode_wrapped_key() {
        let nonce = vec![0u8; 12];
        let wrapped = vec![1u8; 48];

        let encoded = encode_wrapped_key(&nonce, &wrapped);
        assert!(!encoded.is_empty());

        // Decode and verify
        use base64::{Engine, engine::general_purpose::STANDARD};
        let decoded = STANDARD.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), 60); // 12 + 48
    }
}
