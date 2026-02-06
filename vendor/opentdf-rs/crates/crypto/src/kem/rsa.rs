//! RSA-OAEP Key Encapsulation Mechanism
//!
//! This module implements RSA-OAEP key wrapping for OpenTDF.
//! SHA-1 is used by default for compatibility with the OpenTDF Go SDK,
//! but SHA-256 is also supported and recommended for new deployments.
//!
//! # Backend Selection
//!
//! - **aws-lc-rs** (default): Constant-time RSA operations, FIPS validated.
//!   This is the recommended backend for production use.
//!
//! - **rustcrypto-provider** (optional): Legacy RustCrypto RSA implementation.
//!   Has RUSTSEC-2023-0071 timing vulnerability. Only use if aws-lc-rs
//!   build requirements cannot be met.

use super::{KemError, KeyEncapsulation};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// OAEP hash algorithm selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OaepHash {
    /// SHA-1 (default for OpenTDF Go SDK compatibility)
    ///
    /// Note: SHA-1 is cryptographically weak but required for interoperability
    /// with existing OpenTDF implementations. Use SHA-256 for new deployments.
    #[default]
    Sha1,

    /// SHA-256 (recommended for new deployments)
    Sha256,
}

/// RSA-OAEP key encapsulation mechanism
pub struct RsaOaepKem {
    /// Hash algorithm for OAEP padding
    pub hash: OaepHash,
}

impl Default for RsaOaepKem {
    fn default() -> Self {
        RsaOaepKem {
            hash: OaepHash::Sha1, // Default to SHA-1 for Go SDK compatibility
        }
    }
}

impl RsaOaepKem {
    /// Create a new RSA-OAEP KEM with the specified hash algorithm
    pub fn new(hash: OaepHash) -> Self {
        RsaOaepKem { hash }
    }

    /// Create with SHA-1 (default, for OpenTDF compatibility)
    pub fn with_sha1() -> Self {
        RsaOaepKem {
            hash: OaepHash::Sha1,
        }
    }

    /// Create with SHA-256 (recommended)
    pub fn with_sha256() -> Self {
        RsaOaepKem {
            hash: OaepHash::Sha256,
        }
    }
}

// ============================================================================
// aws-lc-rs backend (default, constant-time)
// ============================================================================
#[cfg(feature = "aws-lc-provider")]
mod aws_lc_impl {
    use super::*;
    use aws_lc_rs::rsa::{
        OAEP_SHA1_MGF1SHA1, OAEP_SHA256_MGF1SHA256, OaepPrivateDecryptingKey,
        OaepPublicEncryptingKey, PrivateDecryptingKey, PublicEncryptingKey,
    };

    /// Parse PEM to DER bytes
    fn pem_to_der(pem_str: &str) -> Result<Vec<u8>, KemError> {
        let pem = pem::parse(pem_str)
            .map_err(|e| KemError::InvalidKey(format!("Failed to parse PEM: {}", e)))?;
        Ok(pem.contents().to_vec())
    }

    impl KeyEncapsulation for RsaOaepKem {
        type PublicKey = String; // PEM-encoded public key
        type PrivateKey = String; // PEM-encoded private key
        type WrappedKey = String; // Base64-encoded ciphertext

        fn wrap(&self, key: &[u8], public_key_pem: &Self::PublicKey) -> Result<String, KemError> {
            // Parse PEM to DER
            let der = pem_to_der(public_key_pem)?;

            // Load public key
            let public_key = PublicEncryptingKey::from_der(&der).map_err(|e| {
                KemError::InvalidKey(format!("Failed to parse RSA public key: {:?}", e))
            })?;

            // Create OAEP encrypting key
            let oaep_key = OaepPublicEncryptingKey::new(public_key)
                .map_err(|e| KemError::InvalidKey(format!("Failed to create OAEP key: {:?}", e)))?;

            // Allocate ciphertext buffer
            let mut ciphertext = vec![0u8; oaep_key.ciphertext_size()];

            // Select algorithm based on hash
            let algorithm = match self.hash {
                OaepHash::Sha1 => &OAEP_SHA1_MGF1SHA1,
                OaepHash::Sha256 => &OAEP_SHA256_MGF1SHA256,
            };

            // Encrypt
            let ciphertext_len = oaep_key
                .encrypt(algorithm, key, &mut ciphertext, None)
                .map_err(|e| KemError::WrapError(format!("RSA-OAEP encryption failed: {:?}", e)))?
                .len();

            ciphertext.truncate(ciphertext_len);

            // Base64 encode for storage in manifest
            Ok(BASE64.encode(&ciphertext))
        }

        fn unwrap(
            &self,
            wrapped_b64: &Self::WrappedKey,
            private_key_pem: &Self::PrivateKey,
        ) -> Result<Vec<u8>, KemError> {
            // Decode base64 wrapped key
            let ciphertext = BASE64
                .decode(wrapped_b64)
                .map_err(|e| KemError::EncodingError(format!("Base64 decode failed: {}", e)))?;

            // Parse PEM to DER
            let der = pem_to_der(private_key_pem)?;

            // Load private key
            let private_key = PrivateDecryptingKey::from_pkcs8(&der).map_err(|e| {
                KemError::InvalidKey(format!("Failed to parse RSA private key: {:?}", e))
            })?;

            // Create OAEP decrypting key
            let oaep_key = OaepPrivateDecryptingKey::new(private_key)
                .map_err(|e| KemError::InvalidKey(format!("Failed to create OAEP key: {:?}", e)))?;

            // Allocate plaintext buffer
            let mut plaintext = vec![0u8; oaep_key.min_output_size()];

            // Select algorithm based on hash
            let algorithm = match self.hash {
                OaepHash::Sha1 => &OAEP_SHA1_MGF1SHA1,
                OaepHash::Sha256 => &OAEP_SHA256_MGF1SHA256,
            };

            // Decrypt
            let plaintext_slice = oaep_key
                .decrypt(algorithm, &ciphertext, &mut plaintext, None)
                .map_err(|e| {
                    KemError::UnwrapError(format!("RSA-OAEP decryption failed: {:?}", e))
                })?;

            Ok(plaintext_slice.to_vec())
        }
    }
}

// ============================================================================
// RustCrypto rsa backend (legacy, has timing vulnerability)
// ============================================================================
#[cfg(all(feature = "rustcrypto-provider", not(feature = "aws-lc-provider")))]
mod rustcrypto_impl {
    use super::*;
    use rand::rngs::OsRng;
    use rsa::{
        Oaep, RsaPrivateKey, RsaPublicKey,
        pkcs8::{DecodePrivateKey, DecodePublicKey},
    };
    use sha1::Sha1;
    use sha2::Sha256;

    impl KeyEncapsulation for RsaOaepKem {
        type PublicKey = String; // PEM-encoded public key
        type PrivateKey = String; // PEM-encoded private key
        type WrappedKey = String; // Base64-encoded ciphertext

        fn wrap(&self, key: &[u8], public_key_pem: &Self::PublicKey) -> Result<String, KemError> {
            // Parse PEM-encoded public key
            let public_key = RsaPublicKey::from_public_key_pem(public_key_pem).map_err(|e| {
                KemError::InvalidKey(format!("Failed to parse RSA public key: {}", e))
            })?;

            // Encrypt with OAEP padding using the selected hash
            let wrapped = match self.hash {
                OaepHash::Sha1 => {
                    let padding = Oaep::new::<Sha1>();
                    public_key.encrypt(&mut OsRng, padding, key).map_err(|e| {
                        KemError::WrapError(format!("RSA-OAEP encryption failed: {}", e))
                    })?
                }
                OaepHash::Sha256 => {
                    let padding = Oaep::new::<Sha256>();
                    public_key.encrypt(&mut OsRng, padding, key).map_err(|e| {
                        KemError::WrapError(format!("RSA-OAEP encryption failed: {}", e))
                    })?
                }
            };

            // Base64 encode for storage in manifest
            Ok(BASE64.encode(&wrapped))
        }

        fn unwrap(
            &self,
            wrapped_b64: &Self::WrappedKey,
            private_key_pem: &Self::PrivateKey,
        ) -> Result<Vec<u8>, KemError> {
            // Decode base64 wrapped key
            let wrapped = BASE64
                .decode(wrapped_b64)
                .map_err(|e| KemError::EncodingError(format!("Base64 decode failed: {}", e)))?;

            // Parse PEM-encoded private key
            let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem).map_err(|e| {
                KemError::InvalidKey(format!("Failed to parse RSA private key: {}", e))
            })?;

            // Decrypt with OAEP padding using the selected hash
            let key = match self.hash {
                OaepHash::Sha1 => {
                    let padding = Oaep::new::<Sha1>();
                    private_key.decrypt(padding, &wrapped).map_err(|e| {
                        KemError::UnwrapError(format!("RSA-OAEP decryption failed: {}", e))
                    })?
                }
                OaepHash::Sha256 => {
                    let padding = Oaep::new::<Sha256>();
                    private_key.decrypt(padding, &wrapped).map_err(|e| {
                        KemError::UnwrapError(format!("RSA-OAEP decryption failed: {}", e))
                    })?
                }
            };

            Ok(key)
        }
    }
}

/// Convenience function to wrap a key with RSA-OAEP (SHA-1 default)
///
/// This function wraps a symmetric payload key with an RSA public key using OAEP padding.
/// It uses SHA-1 by default for compatibility with the OpenTDF Go SDK.
///
/// # Arguments
///
/// * `payload_key` - The symmetric key to wrap (typically 32 bytes for AES-256)
/// * `kas_public_key_pem` - PEM-encoded RSA public key from KAS
///
/// # Returns
///
/// Base64-encoded wrapped key ready for inclusion in TDF manifest
///
/// # Example
///
/// ```no_run
/// use opentdf_crypto::kem::rsa::wrap_key_with_rsa_oaep;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let payload_key = &[0u8; 32]; // Your AES-256 key
/// let kas_public_key_pem = "-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----";
///
/// let wrapped_key = wrap_key_with_rsa_oaep(payload_key, kas_public_key_pem)?;
/// # Ok(())
/// # }
/// ```
#[cfg(any(feature = "aws-lc-provider", feature = "rustcrypto-provider"))]
pub fn wrap_key_with_rsa_oaep(
    payload_key: &[u8],
    kas_public_key_pem: &str,
) -> Result<String, KemError> {
    let kem = RsaOaepKem::default();
    kem.wrap(payload_key, &kas_public_key_pem.to_string())
}

#[cfg(test)]
#[cfg(feature = "aws-lc-provider")]
mod tests {
    use super::*;
    use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der};
    use aws_lc_rs::rsa::{KeySize, PrivateDecryptingKey};

    fn generate_test_keypair() -> (String, String) {
        // Generate RSA-2048 key pair using aws-lc-rs
        let private_key = PrivateDecryptingKey::generate(KeySize::Rsa2048).unwrap();
        let public_key = private_key.public_key();

        // Export to DER
        let private_der = AsDer::<Pkcs8V1Der>::as_der(&private_key).unwrap();
        let public_der = AsDer::<PublicKeyX509Der>::as_der(&public_key).unwrap();

        // Convert to PEM
        let private_pem = pem::Pem::new("PRIVATE KEY", private_der.as_ref());
        let public_pem = pem::Pem::new("PUBLIC KEY", public_der.as_ref());

        (pem::encode(&public_pem), pem::encode(&private_pem))
    }

    #[test]
    fn test_rsa_oaep_roundtrip_sha1() {
        let (public_pem, private_pem) = generate_test_keypair();
        let kem = RsaOaepKem::with_sha1();

        let key = b"test_payload_key_32_bytes_long!";
        let wrapped = kem.wrap(key, &public_pem).unwrap();
        let unwrapped = kem.unwrap(&wrapped, &private_pem).unwrap();

        assert_eq!(key, unwrapped.as_slice());
    }

    #[test]
    fn test_rsa_oaep_roundtrip_sha256() {
        let (public_pem, private_pem) = generate_test_keypair();
        let kem = RsaOaepKem::with_sha256();

        let key = b"test_payload_key_32_bytes_long!";
        let wrapped = kem.wrap(key, &public_pem).unwrap();
        let unwrapped = kem.unwrap(&wrapped, &private_pem).unwrap();

        assert_eq!(key, unwrapped.as_slice());
    }

    #[test]
    fn test_convenience_function() {
        let (public_pem, _private_pem) = generate_test_keypair();
        let key = b"test_payload_key_32_bytes_long!";

        let wrapped = wrap_key_with_rsa_oaep(key, &public_pem).unwrap();
        assert!(!wrapped.is_empty());
        // Should be base64 encoded
        assert!(BASE64.decode(&wrapped).is_ok());
    }

    #[test]
    fn test_rsa_oaep_varied_payload_sizes() {
        // Test RSA-OAEP with different key sizes (AES-128, AES-192, AES-256)
        let (public_pem, private_pem) = generate_test_keypair();
        let kem = RsaOaepKem::with_sha256();

        // 16-byte key (AES-128)
        let key_16 = [0x42u8; 16];
        let wrapped = kem.wrap(&key_16, &public_pem).unwrap();
        let unwrapped = kem.unwrap(&wrapped, &private_pem).unwrap();
        assert_eq!(key_16.as_slice(), unwrapped.as_slice());

        // 24-byte key (AES-192)
        let key_24 = [0x43u8; 24];
        let wrapped = kem.wrap(&key_24, &public_pem).unwrap();
        let unwrapped = kem.unwrap(&wrapped, &private_pem).unwrap();
        assert_eq!(key_24.as_slice(), unwrapped.as_slice());

        // 32-byte key (AES-256)
        let key_32 = [0x44u8; 32];
        let wrapped = kem.wrap(&key_32, &public_pem).unwrap();
        let unwrapped = kem.unwrap(&wrapped, &private_pem).unwrap();
        assert_eq!(key_32.as_slice(), unwrapped.as_slice());
    }

    #[test]
    fn test_rsa_invalid_pem_format() {
        // Test handling of malformed PEM input
        let kem = RsaOaepKem::default();
        let key = b"test_payload_key_32_bytes_long!";

        // Completely invalid PEM
        let result = kem.wrap(key, &"not a valid pem".to_string());
        assert!(result.is_err());

        // Truncated PEM
        let result = kem.wrap(key, &"-----BEGIN PUBLIC KEY-----\nAAAA".to_string());
        assert!(result.is_err());

        // Empty PEM
        let result = kem.wrap(key, &String::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_rsa_unwrap_invalid_ciphertext() {
        // Test handling of corrupted ciphertext
        let (public_pem, private_pem) = generate_test_keypair();
        let kem = RsaOaepKem::default();
        let key = b"test_payload_key_32_bytes_long!";

        // Wrap a key
        let wrapped = kem.wrap(key, &public_pem).unwrap();

        // Corrupt the ciphertext
        let mut corrupted_bytes = BASE64.decode(&wrapped).unwrap();
        corrupted_bytes[10] ^= 0xFF; // Flip bits in ciphertext
        let corrupted = BASE64.encode(&corrupted_bytes);

        // Should fail to unwrap
        let result = kem.unwrap(&corrupted, &private_pem);
        assert!(result.is_err());
    }
}

#[cfg(test)]
#[cfg(all(feature = "rustcrypto-provider", not(feature = "aws-lc-provider")))]
mod tests_rustcrypto {
    use super::*;
    use pkcs8::EncodePrivateKey;
    use rand::rngs::OsRng;
    use rsa::pkcs8::EncodePublicKey;
    use rsa::{RsaPrivateKey, RsaPublicKey};

    fn generate_test_keypair() -> (String, String) {
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = RsaPublicKey::from(&private_key);

        let private_pem = private_key.to_pkcs8_pem(pkcs8::LineEnding::LF).unwrap();
        let public_pem = public_key.to_public_key_pem(pkcs8::LineEnding::LF).unwrap();

        (public_pem, private_pem.to_string())
    }

    #[test]
    fn test_rsa_oaep_roundtrip_sha1() {
        let (public_pem, private_pem) = generate_test_keypair();
        let kem = RsaOaepKem::with_sha1();

        let key = b"test_payload_key_32_bytes_long!";
        let wrapped = kem.wrap(key, &public_pem).unwrap();
        let unwrapped = kem.unwrap(&wrapped, &private_pem).unwrap();

        assert_eq!(key, unwrapped.as_slice());
    }

    #[test]
    fn test_rsa_oaep_roundtrip_sha256() {
        let (public_pem, private_pem) = generate_test_keypair();
        let kem = RsaOaepKem::with_sha256();

        let key = b"test_payload_key_32_bytes_long!";
        let wrapped = kem.wrap(key, &public_pem).unwrap();
        let unwrapped = kem.unwrap(&wrapped, &private_pem).unwrap();

        assert_eq!(key, unwrapped.as_slice());
    }

    #[test]
    fn test_convenience_function() {
        let (public_pem, _private_pem) = generate_test_keypair();
        let key = b"test_payload_key_32_bytes_long!";

        let wrapped = wrap_key_with_rsa_oaep(key, &public_pem).unwrap();
        assert!(!wrapped.is_empty());
        // Should be base64 encoded
        assert!(BASE64.decode(&wrapped).is_ok());
    }
}
