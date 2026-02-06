//! Elliptic Curve Key Encapsulation for NanoTDF
//!
//! This module implements ECDH-based key derivation for NanoTDF following the spec:
//! 1. ECDH key agreement between ephemeral and recipient keys
//! 2. HKDF-SHA256 key derivation with NanoTDF-specific salt
//! 3. Derive AES-256 keys for payload encryption

use super::{KemError, KeyEncapsulation};
use crate::types::AesKey;
use zeroize::Zeroizing;

#[cfg(feature = "kem-ec")]
use hkdf::Hkdf;

#[cfg(feature = "kem-ec")]
use sha2::Sha256;

/// NanoTDF HKDF salt: SHA256(MAGIC_NUMBER + VERSION) = SHA256(0x4C314C)
/// This is defined in the NanoTDF spec section 4
pub const NANOTDF_HKDF_SALT: [u8; 32] = [
    0x3d, 0xe3, 0xca, 0x1e, 0x50, 0xcf, 0x62, 0xd8, 0xb6, 0xab, 0xa6, 0x03, 0xa9, 0x6f, 0xca, 0x67,
    0x61, 0x38, 0x7a, 0x7a, 0xc8, 0x6c, 0x3d, 0x3a, 0xfe, 0x85, 0xae, 0x2d, 0x18, 0x12, 0xed, 0xfc,
];

/// Elliptic curve selection for ECDH
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcCurve {
    /// NIST P-256 (secp256r1) - Primary curve for NanoTDF
    P256,
    /// NIST P-384 (secp384r1)
    P384,
    /// NIST P-521 (secp521r1)
    P521,
    /// secp256k1 (Bitcoin curve)
    Secp256k1,
}

impl EcCurve {
    /// Get the size of compressed public keys for this curve
    pub fn public_key_size(self) -> usize {
        match self {
            EcCurve::P256 => 33,      // 1 byte prefix + 32 bytes
            EcCurve::P384 => 49,      // 1 byte prefix + 48 bytes
            EcCurve::P521 => 67,      // 1 byte prefix + 66 bytes
            EcCurve::Secp256k1 => 33, // 1 byte prefix + 32 bytes
        }
    }

    /// Get the size of ECDSA signatures for this curve (r + s)
    pub fn signature_size(self) -> usize {
        match self {
            EcCurve::P256 => 64,      // r (32 bytes) + s (32 bytes)
            EcCurve::P384 => 96,      // r (48 bytes) + s (48 bytes)
            EcCurve::P521 => 132,     // r (66 bytes) + s (66 bytes)
            EcCurve::Secp256k1 => 64, // r (32 bytes) + s (32 bytes)
        }
    }
}

/// ECDH key encapsulation mechanism (NanoTDF)
///
/// This implementation uses ECDH key agreement followed by HKDF key derivation.
///
/// # Protocol Flow
///
/// 1. Generate ephemeral EC key pair
/// 2. Perform ECDH with recipient's public key → shared secret
/// 3. Derive encryption key using HKDF-SHA256 with NanoTDF salt
/// 4. Return ephemeral public key (compressed format)
///
/// # NanoTDF Spec Compliance
///
/// - Uses HKDF-SHA256 with salt = SHA256(MAGIC_NUMBER + VERSION)
/// - Empty info parameter for HKDF
/// - Derives 32-byte AES keys
pub struct EcdhKem {
    /// Elliptic curve to use
    pub curve: EcCurve,
}

impl EcdhKem {
    /// Create a new ECDH KEM with specified curve
    pub fn new(curve: EcCurve) -> Self {
        Self { curve }
    }
}

impl Default for EcdhKem {
    fn default() -> Self {
        EcdhKem {
            curve: EcCurve::P256,
        }
    }
}

// Derive AES key from ECDH shared secret using HKDF-SHA256
#[cfg(feature = "kem-ec")]
fn derive_key_from_shared_secret(shared_secret: &[u8]) -> Result<AesKey, KemError> {
    // Use HKDF with NanoTDF-specific salt and empty info
    let hkdf = Hkdf::<Sha256>::new(Some(&NANOTDF_HKDF_SALT), shared_secret);

    // Derive 32 bytes for AES-256
    let mut okm = Zeroizing::new([0u8; 32]);
    hkdf.expand(&[], okm.as_mut())
        .map_err(|_| KemError::KeyDerivationFailed)?;

    AesKey::from_slice(okm.as_ref()).map_err(|_| KemError::KeyDerivationFailed)
}

// P-256 implementation
#[cfg(feature = "kem-ec")]
mod p256_impl {
    use super::*;
    use p256::ecdh::EphemeralSecret;
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use p256::{PublicKey, SecretKey};
    use pkcs8::DecodePrivateKey;
    use rand::rngs::OsRng;

    pub fn derive_key_with_ephemeral(
        recipient_public_key: &[u8],
    ) -> Result<(AesKey, Vec<u8>), KemError> {
        // Parse recipient's public key
        let recipient_key = PublicKey::from_sec1_bytes(recipient_public_key)
            .map_err(|_| KemError::InvalidPublicKey)?;

        // Generate ephemeral secret
        let ephemeral_secret = EphemeralSecret::random(&mut OsRng);

        // Get ephemeral public key (compressed)
        let ephemeral_public = ephemeral_secret.public_key();
        let ephemeral_public_bytes = ephemeral_public.to_encoded_point(true).as_bytes().to_vec();

        // Perform ECDH
        let shared_secret = ephemeral_secret.diffie_hellman(&recipient_key);

        // Derive key using HKDF
        let key = derive_key_from_shared_secret(shared_secret.raw_secret_bytes())?;

        Ok((key, ephemeral_public_bytes))
    }

    pub fn derive_key_with_private(
        private_key: &[u8],
        ephemeral_public_key: &[u8],
    ) -> Result<AesKey, KemError> {
        // Parse private key
        let secret = SecretKey::from_sec1_der(private_key)
            .or_else(|_| SecretKey::from_pkcs8_der(private_key))
            .map_err(|_| KemError::InvalidPrivateKey)?;

        // Parse ephemeral public key
        let ephemeral_pub = PublicKey::from_sec1_bytes(ephemeral_public_key)
            .map_err(|_| KemError::InvalidPublicKey)?;

        // Perform ECDH
        let shared_secret =
            p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), ephemeral_pub.as_affine());

        // Derive key using HKDF
        derive_key_from_shared_secret(shared_secret.raw_secret_bytes())
    }
}

// P-384 implementation
#[cfg(feature = "kem-ec")]
mod p384_impl {
    use super::*;
    use p384::ecdh::EphemeralSecret;
    use p384::elliptic_curve::sec1::ToEncodedPoint;
    use p384::{PublicKey, SecretKey};
    use pkcs8::DecodePrivateKey;
    use rand::rngs::OsRng;

    pub fn derive_key_with_ephemeral(
        recipient_public_key: &[u8],
    ) -> Result<(AesKey, Vec<u8>), KemError> {
        let recipient_key = PublicKey::from_sec1_bytes(recipient_public_key)
            .map_err(|_| KemError::InvalidPublicKey)?;

        let ephemeral_secret = EphemeralSecret::random(&mut OsRng);
        let ephemeral_public = ephemeral_secret.public_key();
        let ephemeral_public_bytes = ephemeral_public.to_encoded_point(true).as_bytes().to_vec();

        let shared_secret = ephemeral_secret.diffie_hellman(&recipient_key);
        let key = derive_key_from_shared_secret(shared_secret.raw_secret_bytes())?;

        Ok((key, ephemeral_public_bytes))
    }

    pub fn derive_key_with_private(
        private_key: &[u8],
        ephemeral_public_key: &[u8],
    ) -> Result<AesKey, KemError> {
        let secret = SecretKey::from_sec1_der(private_key)
            .or_else(|_| SecretKey::from_pkcs8_der(private_key))
            .map_err(|_| KemError::InvalidPrivateKey)?;

        let ephemeral_pub = PublicKey::from_sec1_bytes(ephemeral_public_key)
            .map_err(|_| KemError::InvalidPublicKey)?;

        let shared_secret =
            p384::ecdh::diffie_hellman(secret.to_nonzero_scalar(), ephemeral_pub.as_affine());

        derive_key_from_shared_secret(shared_secret.raw_secret_bytes())
    }
}

// P-521 implementation
#[cfg(feature = "kem-ec")]
mod p521_impl {
    use super::*;
    use p521::ecdh::EphemeralSecret;
    use p521::elliptic_curve::sec1::ToEncodedPoint;
    use p521::{PublicKey, SecretKey};
    use pkcs8::DecodePrivateKey;
    use rand::rngs::OsRng;

    pub fn derive_key_with_ephemeral(
        recipient_public_key: &[u8],
    ) -> Result<(AesKey, Vec<u8>), KemError> {
        let recipient_key = PublicKey::from_sec1_bytes(recipient_public_key)
            .map_err(|_| KemError::InvalidPublicKey)?;

        let ephemeral_secret = EphemeralSecret::random(&mut OsRng);
        let ephemeral_public = ephemeral_secret.public_key();
        let ephemeral_public_bytes = ephemeral_public.to_encoded_point(true).as_bytes().to_vec();

        let shared_secret = ephemeral_secret.diffie_hellman(&recipient_key);
        let key = derive_key_from_shared_secret(shared_secret.raw_secret_bytes())?;

        Ok((key, ephemeral_public_bytes))
    }

    pub fn derive_key_with_private(
        private_key: &[u8],
        ephemeral_public_key: &[u8],
    ) -> Result<AesKey, KemError> {
        let secret = SecretKey::from_sec1_der(private_key)
            .or_else(|_| SecretKey::from_pkcs8_der(private_key))
            .map_err(|_| KemError::InvalidPrivateKey)?;

        let ephemeral_pub = PublicKey::from_sec1_bytes(ephemeral_public_key)
            .map_err(|_| KemError::InvalidPublicKey)?;

        let shared_secret =
            p521::ecdh::diffie_hellman(secret.to_nonzero_scalar(), ephemeral_pub.as_affine());

        derive_key_from_shared_secret(shared_secret.raw_secret_bytes())
    }
}

// secp256k1 implementation
#[cfg(feature = "kem-ec")]
mod k256_impl {
    use super::*;
    use k256::ecdh::EphemeralSecret;
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    use k256::{PublicKey, SecretKey};
    use pkcs8::DecodePrivateKey;
    use rand::rngs::OsRng;

    pub fn derive_key_with_ephemeral(
        recipient_public_key: &[u8],
    ) -> Result<(AesKey, Vec<u8>), KemError> {
        let recipient_key = PublicKey::from_sec1_bytes(recipient_public_key)
            .map_err(|_| KemError::InvalidPublicKey)?;

        let ephemeral_secret = EphemeralSecret::random(&mut OsRng);
        let ephemeral_public = ephemeral_secret.public_key();
        let ephemeral_public_bytes = ephemeral_public.to_encoded_point(true).as_bytes().to_vec();

        let shared_secret = ephemeral_secret.diffie_hellman(&recipient_key);
        let key = derive_key_from_shared_secret(shared_secret.raw_secret_bytes())?;

        Ok((key, ephemeral_public_bytes))
    }

    pub fn derive_key_with_private(
        private_key: &[u8],
        ephemeral_public_key: &[u8],
    ) -> Result<AesKey, KemError> {
        let secret = SecretKey::from_sec1_der(private_key)
            .or_else(|_| SecretKey::from_pkcs8_der(private_key))
            .map_err(|_| KemError::InvalidPrivateKey)?;

        let ephemeral_pub = PublicKey::from_sec1_bytes(ephemeral_public_key)
            .map_err(|_| KemError::InvalidPublicKey)?;

        let shared_secret =
            k256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), ephemeral_pub.as_affine());

        derive_key_from_shared_secret(shared_secret.raw_secret_bytes())
    }
}

impl EcdhKem {
    /// Derive a key using recipient's public key
    ///
    /// Returns: (derived_key, ephemeral_public_key_compressed)
    ///
    /// This is used for encryption - generates a new ephemeral key pair
    /// and returns the derived key plus the ephemeral public key to include
    /// in the NanoTDF header.
    #[cfg(feature = "kem-ec")]
    pub fn derive_key_with_ephemeral(
        &self,
        recipient_public_key: &[u8],
    ) -> Result<(AesKey, Vec<u8>), KemError> {
        match self.curve {
            EcCurve::P256 => p256_impl::derive_key_with_ephemeral(recipient_public_key),
            EcCurve::P384 => p384_impl::derive_key_with_ephemeral(recipient_public_key),
            EcCurve::P521 => p521_impl::derive_key_with_ephemeral(recipient_public_key),
            EcCurve::Secp256k1 => k256_impl::derive_key_with_ephemeral(recipient_public_key),
        }
    }

    /// Derive a key using private key and ephemeral public key
    ///
    /// This is used for decryption - uses the recipient's private key
    /// and the ephemeral public key from the NanoTDF header to derive
    /// the same key used for encryption.
    #[cfg(feature = "kem-ec")]
    pub fn derive_key_with_private(
        &self,
        private_key: &[u8],
        ephemeral_public_key: &[u8],
    ) -> Result<AesKey, KemError> {
        match self.curve {
            EcCurve::P256 => p256_impl::derive_key_with_private(private_key, ephemeral_public_key),
            EcCurve::P384 => p384_impl::derive_key_with_private(private_key, ephemeral_public_key),
            EcCurve::P521 => p521_impl::derive_key_with_private(private_key, ephemeral_public_key),
            EcCurve::Secp256k1 => {
                k256_impl::derive_key_with_private(private_key, ephemeral_public_key)
            }
        }
    }

    #[cfg(not(feature = "kem-ec"))]
    pub fn derive_key_with_ephemeral(
        &self,
        _recipient_public_key: &[u8],
    ) -> Result<(AesKey, Vec<u8>), KemError> {
        Err(KemError::UnsupportedAlgorithm(
            "ECDH KEM requires 'kem-ec' feature".to_string(),
        ))
    }

    #[cfg(not(feature = "kem-ec"))]
    pub fn derive_key_with_private(
        &self,
        _private_key: &[u8],
        _ephemeral_public_key: &[u8],
    ) -> Result<AesKey, KemError> {
        Err(KemError::UnsupportedAlgorithm(
            "ECDH KEM requires 'kem-ec' feature".to_string(),
        ))
    }
}

impl KeyEncapsulation for EcdhKem {
    type PublicKey = Vec<u8>; // Compressed or uncompressed EC point
    type PrivateKey = Vec<u8>; // DER-encoded
    type WrappedKey = Vec<u8>; // Ephemeral public key (compressed)

    fn wrap(&self, _key: &[u8], _public_key: &Self::PublicKey) -> Result<Vec<u8>, KemError> {
        // Note: For NanoTDF, we don't "wrap" an existing key
        // Instead, we derive a key from ECDH and return the ephemeral public key
        // This method is not used in NanoTDF flow - use derive_key_with_ephemeral instead
        Err(KemError::UnsupportedAlgorithm(
            "For NanoTDF, use derive_key_with_ephemeral() instead of wrap()".to_string(),
        ))
    }

    fn unwrap(
        &self,
        _wrapped: &Self::WrappedKey,
        _private_key: &Self::PrivateKey,
    ) -> Result<Vec<u8>, KemError> {
        // Note: For NanoTDF, we don't "unwrap" a key
        // Instead, we derive a key from ECDH using the ephemeral public key
        // This method is not used in NanoTDF flow - use derive_key_with_private instead
        Err(KemError::UnsupportedAlgorithm(
            "For NanoTDF, use derive_key_with_private() instead of unwrap()".to_string(),
        ))
    }
}

// ============================================================================
// EC Key Wrapping for TDF-JSON/CBOR (ECIES: ECDH + HKDF + AES-GCM)
// ============================================================================

/// Result of EC key wrapping containing wrapped key and ephemeral public key
#[derive(Debug, Clone)]
pub struct EcWrappedKeyResult {
    /// The wrapped symmetric key (base64: nonce + ciphertext + tag)
    pub wrapped_key: String,
    /// The ephemeral public key in PEM format
    pub ephemeral_public_key: String,
}

/// Wrap a symmetric key using EC (ECIES: ECDH + HKDF + AES-GCM)
///
/// This generates an ephemeral key pair and uses ECDH with the recipient's
/// public key to derive a wrapping key, then wraps the symmetric key with AES-GCM.
///
/// # Arguments
///
/// * `recipient_public_key_pem` - The recipient's EC public key in PEM format
/// * `symmetric_key` - The symmetric key to wrap (32 bytes for AES-256)
///
/// # Returns
///
/// `EcWrappedKeyResult` containing the wrapped key (base64) and ephemeral public key (PEM)
#[cfg(feature = "kem-ec")]
pub fn wrap_key_with_ec(
    recipient_public_key_pem: &str,
    symmetric_key: &[u8],
) -> Result<EcWrappedKeyResult, KemError> {
    use aes_gcm::{
        Aes256Gcm, Nonce,
        aead::{Aead, KeyInit},
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use hkdf::Hkdf;
    use p256::PublicKey;
    use p256::ecdh::EphemeralSecret;
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use rand::rngs::OsRng;
    use sha2::Sha256;

    // Parse PEM to get DER bytes
    let pem_lines: Vec<&str> = recipient_public_key_pem.lines().collect();
    let base64_content: String = pem_lines
        .iter()
        .filter(|line| !line.starts_with("-----"))
        .map(|s| s.trim())
        .collect();
    let der_bytes = BASE64
        .decode(&base64_content)
        .map_err(|e| KemError::InvalidKey(format!("Invalid PEM encoding: {}", e)))?;

    // Parse recipient's public key from DER (SPKI format)
    let recipient_key = PublicKey::from_sec1_bytes(&der_bytes)
        .or_else(|_| {
            // Try parsing as SPKI (SubjectPublicKeyInfo) format using proper ASN.1 parsing
            use spki::SubjectPublicKeyInfoRef;
            let spki = SubjectPublicKeyInfoRef::try_from(der_bytes.as_slice())
                .map_err(|_| p256::elliptic_curve::Error)?;
            // Extract the raw public key bytes from the SPKI structure
            PublicKey::from_sec1_bytes(spki.subject_public_key.raw_bytes())
        })
        .map_err(|_| KemError::InvalidPublicKey)?;

    // Generate ephemeral key pair
    let ephemeral_secret = EphemeralSecret::random(&mut OsRng);
    let ephemeral_public = ephemeral_secret.public_key();

    // Perform ECDH
    let shared_secret = ephemeral_secret.diffie_hellman(&recipient_key);

    // Derive wrapping key using HKDF-SHA256 (empty salt and info for TDF compatibility)
    let hkdf = Hkdf::<Sha256>::new(None, shared_secret.raw_secret_bytes());
    let mut wrap_key = Zeroizing::new([0u8; 32]);
    hkdf.expand(&[], wrap_key.as_mut())
        .map_err(|_| KemError::KeyDerivationFailed)?;

    // Wrap the symmetric key with AES-GCM
    let cipher = Aes256Gcm::new_from_slice(wrap_key.as_ref())
        .map_err(|_| KemError::WrapError("Invalid key".into()))?;

    // Generate random nonce
    let mut nonce_bytes = [0u8; 12];
    use rand::RngCore;
    OsRng.fill_bytes(&mut nonce_bytes);
    #[allow(deprecated)]
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, symmetric_key)
        .map_err(|e| KemError::WrapError(format!("AES-GCM encryption failed: {}", e)))?;

    // Format: nonce + ciphertext + tag (combined by AES-GCM)
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    let wrapped_key = BASE64.encode(&combined);

    // Convert ephemeral public key to compressed SEC1 format (33 bytes for P-256)
    // Much smaller than PEM (~140 bytes) or uncompressed (65 bytes)
    let ephemeral_compressed = ephemeral_public.to_encoded_point(true);
    let ephemeral_b64 = BASE64.encode(ephemeral_compressed.as_bytes());

    Ok(EcWrappedKeyResult {
        wrapped_key,
        ephemeral_public_key: ephemeral_b64,
    })
}

/// Unwrap a symmetric key using EC (ECIES: ECDH + HKDF + AES-GCM)
#[cfg(feature = "kem-ec")]
pub fn unwrap_key_with_ec(
    private_key_pem: &str,
    wrapped_key_base64: &str,
    ephemeral_public_key_pem: &str,
) -> Result<Vec<u8>, KemError> {
    use aes_gcm::{
        Aes256Gcm, Nonce,
        aead::{Aead, KeyInit},
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use hkdf::Hkdf;
    use p256::{PublicKey, SecretKey};
    use pkcs8::DecodePrivateKey;
    use sha2::Sha256;

    // Parse private key from PEM
    let pem_lines: Vec<&str> = private_key_pem.lines().collect();
    let base64_content: String = pem_lines
        .iter()
        .filter(|line| !line.starts_with("-----"))
        .map(|s| s.trim())
        .collect();
    let der_bytes = BASE64
        .decode(&base64_content)
        .map_err(|e| KemError::InvalidKey(format!("Invalid PEM encoding: {}", e)))?;

    let private_key = SecretKey::from_pkcs8_der(&der_bytes)
        .or_else(|_| SecretKey::from_sec1_der(&der_bytes))
        .map_err(|_| KemError::InvalidPrivateKey)?;

    // Parse ephemeral public key - supports both base64 SEC1 and PEM formats
    let ephemeral_bytes = if ephemeral_public_key_pem.contains("-----BEGIN") {
        // PEM format (legacy)
        let pem_lines: Vec<&str> = ephemeral_public_key_pem.lines().collect();
        let base64_content: String = pem_lines
            .iter()
            .filter(|line| !line.starts_with("-----"))
            .map(|s| s.trim())
            .collect();
        BASE64
            .decode(&base64_content)
            .map_err(|e| KemError::InvalidKey(format!("Invalid ephemeral PEM encoding: {}", e)))?
    } else {
        // Base64 SEC1 format (compressed or uncompressed)
        BASE64
            .decode(ephemeral_public_key_pem)
            .map_err(|e| KemError::InvalidKey(format!("Invalid ephemeral key encoding: {}", e)))?
    };

    let ephemeral_public =
        PublicKey::from_sec1_bytes(&ephemeral_bytes).map_err(|_| KemError::InvalidPublicKey)?;

    // Perform ECDH
    let shared_secret = p256::ecdh::diffie_hellman(
        private_key.to_nonzero_scalar(),
        ephemeral_public.as_affine(),
    );

    // Derive wrapping key using HKDF-SHA256
    let hkdf = Hkdf::<Sha256>::new(None, shared_secret.raw_secret_bytes());
    let mut wrap_key = Zeroizing::new([0u8; 32]);
    hkdf.expand(&[], wrap_key.as_mut())
        .map_err(|_| KemError::KeyDerivationFailed)?;

    // Decode wrapped key
    let wrapped_data = BASE64
        .decode(wrapped_key_base64)
        .map_err(|e| KemError::UnwrapError(format!("Invalid wrapped key encoding: {}", e)))?;

    // Parse: nonce (12) + ciphertext (32) + tag (16) = 60 bytes minimum for AES-256 key
    if wrapped_data.len() < 60 {
        return Err(KemError::UnwrapError("Wrapped key too short".into()));
    }

    let nonce_bytes = &wrapped_data[..12];
    let ciphertext = &wrapped_data[12..];

    // Unwrap with AES-GCM
    let cipher = Aes256Gcm::new_from_slice(wrap_key.as_ref())
        .map_err(|_| KemError::UnwrapError("Invalid wrap key".into()))?;
    #[allow(deprecated)]
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| KemError::UnwrapError(format!("AES-GCM decryption failed: {}", e)))?;

    Ok(plaintext)
}

#[cfg(all(test, feature = "kas"))]
mod tests {
    use super::*;
    use p256::SecretKey;
    use p256::pkcs8::EncodePrivateKey;
    use rand::rngs::OsRng;

    #[test]
    fn test_p256_key_derivation_roundtrip() {
        // Generate a recipient key pair
        let recipient_secret = SecretKey::random(&mut OsRng);
        let recipient_public = recipient_secret.public_key();
        let recipient_public_bytes = recipient_public.to_sec1_bytes();

        // Derive key with ephemeral (encryption side)
        let kem = EcdhKem::new(EcCurve::P256);
        let (derived_key1, ephemeral_public) = kem
            .derive_key_with_ephemeral(&recipient_public_bytes)
            .unwrap();

        // Derive key with private key (decryption side)
        let recipient_private_bytes = recipient_secret.to_pkcs8_der().unwrap();
        let derived_key2 = kem
            .derive_key_with_private(recipient_private_bytes.as_bytes(), &ephemeral_public)
            .unwrap();

        // Keys should match
        assert_eq!(derived_key1.as_slice(), derived_key2.as_slice());

        // Ephemeral public key should be compressed (33 bytes for P-256)
        assert_eq!(ephemeral_public.len(), 33);
    }

    #[test]
    fn test_hkdf_salt() {
        // Verify the HKDF salt matches the spec
        use sha2::{Digest, Sha256};
        let magic_and_version = [0x4C, 0x31, 0x4C];
        let expected_salt = Sha256::digest(&magic_and_version);
        assert_eq!(NANOTDF_HKDF_SALT, expected_salt.as_slice());
    }

    #[test]
    fn test_all_curves() {
        // Test that all curves can derive keys successfully
        let curves = [
            EcCurve::P256,
            EcCurve::P384,
            EcCurve::P521,
            EcCurve::Secp256k1,
        ];

        for curve in curves {
            let kem = EcdhKem::new(curve);

            // Generate a dummy public key for testing
            // (In real usage, this would be a proper recipient public key)
            let (_, dummy_pubkey) = match curve {
                EcCurve::P256 => {
                    let secret = p256::SecretKey::random(&mut OsRng);
                    let pubkey = secret.public_key().to_sec1_bytes().to_vec();
                    let private = secret.to_pkcs8_der().unwrap();
                    (private.to_bytes().to_vec(), pubkey)
                }
                EcCurve::P384 => {
                    let secret = p384::SecretKey::random(&mut OsRng);
                    let pubkey = secret.public_key().to_sec1_bytes().to_vec();
                    let private = secret.to_pkcs8_der().unwrap();
                    (private.to_bytes().to_vec(), pubkey)
                }
                EcCurve::P521 => {
                    let secret = p521::SecretKey::random(&mut OsRng);
                    let pubkey = secret.public_key().to_sec1_bytes().to_vec();
                    let private = secret.to_pkcs8_der().unwrap();
                    (private.to_bytes().to_vec(), pubkey)
                }
                EcCurve::Secp256k1 => {
                    let secret = k256::SecretKey::random(&mut OsRng);
                    let pubkey = secret.public_key().to_sec1_bytes().to_vec();
                    let private = secret.to_pkcs8_der().unwrap();
                    (private.to_bytes().to_vec(), pubkey)
                }
            };

            // Should be able to derive a key
            let result = kem.derive_key_with_ephemeral(&dummy_pubkey);
            assert!(result.is_ok(), "Failed to derive key for {:?}", curve);

            let (key, ephemeral) = result.unwrap();
            assert_eq!(key.as_slice().len(), 32); // Should be 32 bytes for AES-256
            assert_eq!(ephemeral.len(), curve.public_key_size());
        }
    }
}
