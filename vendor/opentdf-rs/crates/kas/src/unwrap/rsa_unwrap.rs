//! RSA (Standard TDF) unwrap operations
//!
//! Provides RSA-OAEP decryption for Standard TDF wrapped keys.
//!
//! # SHA-1 Usage in RSA-OAEP
//!
//! This module uses SHA-1 as the hash function for RSA-OAEP padding. This is
//! intentional for compatibility with the OpenTDF specification and existing
//! TDF files created by other OpenTDF implementations.
//!
//! **Security Note**: SHA-1's collision vulnerabilities do not affect OAEP
//! security. OAEP requires only preimage resistance, which SHA-1 still
//! provides. The use of SHA-1 here is safe and follows the OpenTDF spec.

use crate::error::KasServerError;
use crate::rewrap::{encode_wrapped_key, rewrap_dek_simple};
use rsa::{Oaep, RsaPrivateKey};
use sha1::Sha1;

/// Full RSA unwrap flow for Standard TDF
///
/// Decrypts RSA-OAEP wrapped DEK and rewraps for client session.
///
/// # Arguments
/// * `wrapped_key_bytes` - RSA-OAEP encrypted DEK (256 bytes for RSA-2048)
/// * `rsa_private_key` - KAS RSA private key
/// * `session_shared_secret` - ECDH shared secret with client
pub fn rsa_unwrap(
    wrapped_key_bytes: &[u8],
    rsa_private_key: &RsaPrivateKey,
    session_shared_secret: &[u8],
) -> Result<String, KasServerError> {
    // Validate wrapped key size (RSA-2048 produces 256-byte ciphertext)
    if wrapped_key_bytes.len() != 256 {
        return Err(KasServerError::InvalidKeySize {
            expected: 256,
            got: wrapped_key_bytes.len(),
        });
    }

    // Unwrap DEK using RSA-OAEP with SHA-1 padding (OpenTDF compatibility)
    let padding = Oaep::new::<Sha1>();
    let dek = rsa_private_key
        .decrypt(padding, wrapped_key_bytes)
        .map_err(|e| KasServerError::RsaError(e.to_string()))?;

    // Re-wrap DEK with session shared secret
    let (nonce, wrapped_dek) = rewrap_dek_simple(&dek, session_shared_secret)?;

    // Encode as base64 (nonce + wrapped_dek)
    Ok(encode_wrapped_key(&nonce, &wrapped_dek))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;
    use rsa::Oaep;
    use sha1::Sha1;

    #[test]
    fn test_rsa_unwrap() {
        // Generate RSA keypair
        let rsa_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let rsa_public = rsa::RsaPublicKey::from(&rsa_private);

        // Create a test DEK and wrap it
        let dek = b"test_data_encryption_key_32byte!";
        let padding = Oaep::new::<Sha1>();
        let wrapped = rsa_public.encrypt(&mut OsRng, padding, dek).unwrap();

        // Session shared secret
        let session_secret = b"test_session_shared_secret__32b!";

        // Perform RSA unwrap
        let result = rsa_unwrap(&wrapped, &rsa_private, session_secret);

        assert!(result.is_ok());
        let encoded = result.unwrap();
        assert!(!encoded.is_empty());

        // Decode and verify structure
        use base64::{Engine, engine::general_purpose::STANDARD};
        let decoded = STANDARD.decode(&encoded).unwrap();
        // 12 (nonce) + 32 (DEK) + 16 (tag) = 60 bytes
        assert_eq!(decoded.len(), 60);
    }

    #[test]
    fn test_rsa_unwrap_invalid_key_size() {
        let rsa_private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
        let session_secret = b"test_session_shared_secret__32b!";

        // Wrong size (128 instead of 256)
        let bad_wrapped = [0u8; 128];
        let result = rsa_unwrap(&bad_wrapped, &rsa_private, session_secret);

        assert!(matches!(result, Err(KasServerError::InvalidKeySize { .. })));
    }
}
