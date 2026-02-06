//! EC (NanoTDF) unwrap operations
//!
//! Provides ECDH key agreement and NanoTDF-compatible rewrap.

use crate::error::KasServerError;
use crate::rewrap::{encode_wrapped_key, rewrap_dek};
use crate::salt::{NanoTdfVersion, compute_nanotdf_salt, detect_nanotdf_version};
use p256::elliptic_curve::point::AffineCoordinates;
use p256::{PublicKey, SecretKey};

/// Performs ECDH key agreement and returns x-coordinate as shared secret
///
/// This matches the behavior of OpenTDFKit's custom_ecdh implementation.
/// The x-coordinate is used as the raw shared secret for HKDF derivation.
pub fn custom_ecdh(
    private_key: &SecretKey,
    public_key: &PublicKey,
) -> Result<Vec<u8>, KasServerError> {
    let scalar = private_key.to_nonzero_scalar();
    let public_key_point = public_key.to_projective();
    let shared_point = (public_key_point * *scalar).to_affine();
    let x_coordinate = shared_point.x();
    Ok(x_coordinate.to_vec())
}

/// Full EC unwrap flow for NanoTDF
///
/// Performs ECDH between KAS private key and TDF ephemeral public key,
/// then rewraps the derived DEK for the client session.
///
/// # Arguments
/// * `header_bytes` - Raw NanoTDF header bytes (used for version detection)
/// * `ephemeral_public_key_bytes` - Compressed P-256 public key (33 bytes)
/// * `kas_private_key` - KAS EC private key
/// * `session_shared_secret` - ECDH shared secret with client
pub fn ec_unwrap(
    header_bytes: &[u8],
    ephemeral_public_key_bytes: &[u8],
    kas_private_key: &SecretKey,
    session_shared_secret: &[u8],
) -> Result<String, KasServerError> {
    // Validate ephemeral key size (compressed P-256 = 33 bytes)
    if ephemeral_public_key_bytes.len() != 33 {
        return Err(KasServerError::InvalidKeySize {
            expected: 33,
            got: ephemeral_public_key_bytes.len(),
        });
    }

    // Parse ephemeral public key
    let ephemeral_public_key = PublicKey::from_sec1_bytes(ephemeral_public_key_bytes)
        .map_err(|e| KasServerError::InvalidPublicKey(e.to_string()))?;

    // Perform ECDH to get DEK shared secret
    let dek_shared_secret = custom_ecdh(kas_private_key, &ephemeral_public_key)?;

    // Detect NanoTDF version and compute appropriate salt
    let salt = match detect_nanotdf_version(header_bytes) {
        Some(version) => compute_nanotdf_salt(version),
        None => compute_nanotdf_salt(NanoTdfVersion::V12), // Default to v1.2
    };

    // Rewrap DEK using NanoTDF-compatible HKDF (empty info per spec)
    let (nonce, wrapped_dek) = rewrap_dek(
        &dek_shared_secret,
        session_shared_secret,
        &salt,
        b"", // Empty info per NanoTDF spec
    )?;

    // Encode as base64 (nonce + wrapped_dek)
    Ok(encode_wrapped_key(&nonce, &wrapped_dek))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdh::EphemeralSecret;
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use rand::rngs::OsRng;

    #[test]
    fn test_custom_ecdh() {
        // Generate two keypairs
        let private1 = SecretKey::random(&mut OsRng);
        let public1 = private1.public_key();

        let private2 = SecretKey::random(&mut OsRng);
        let public2 = private2.public_key();

        // ECDH should produce same shared secret both ways
        let shared1 = custom_ecdh(&private1, &public2).unwrap();
        let shared2 = custom_ecdh(&private2, &public1).unwrap();

        assert_eq!(shared1, shared2);
        assert_eq!(shared1.len(), 32); // P-256 x-coordinate is 32 bytes
    }

    #[test]
    fn test_ec_unwrap() {
        // Generate KAS keypair
        let kas_private = SecretKey::random(&mut OsRng);

        // Generate ephemeral keypair (simulating TDF client)
        let ephemeral = EphemeralSecret::random(&mut OsRng);
        let ephemeral_public = PublicKey::from(&ephemeral);
        let ephemeral_public_bytes = ephemeral_public.to_encoded_point(true);

        // Generate session shared secret (simulating client-KAS session)
        let session_secret = b"test_session_shared_secret__32b!";

        // Create NanoTDF v1.2 header
        let header = b"L1L\x00\x00"; // Minimal header with v1.2 magic

        // Perform EC unwrap
        let result = ec_unwrap(
            header,
            ephemeral_public_bytes.as_bytes(),
            &kas_private,
            session_secret,
        );

        assert!(result.is_ok());
        let encoded = result.unwrap();
        assert!(!encoded.is_empty());

        // Decode and verify structure
        use base64::{Engine, engine::general_purpose::STANDARD};
        let decoded = STANDARD.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), 60); // 12 (nonce) + 32 (DEK) + 16 (tag)
    }

    #[test]
    fn test_ec_unwrap_invalid_key_size() {
        let kas_private = SecretKey::random(&mut OsRng);
        let session_secret = b"test_session_shared_secret__32b!";
        let header = b"L1L\x00\x00";

        // Wrong key size (32 instead of 33)
        let bad_key = [0u8; 32];
        let result = ec_unwrap(header, &bad_key, &kas_private, session_secret);

        assert!(matches!(result, Err(KasServerError::InvalidKeySize { .. })));
    }
}
