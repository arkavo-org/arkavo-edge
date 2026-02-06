//! OpenTDF Cryptographic Operations
//!
//! This crate provides cryptographic operations for OpenTDF with security hardening:
//! - Zeroizing key types that automatically clear memory
//! - Constant-time HMAC verification
//! - KEM abstractions for RSA, EC, and future post-quantum algorithms
//! - Segment-based encryption for standard TDF
//! - NanoTDF API structure (implementation pending)
//!
//! # Security Features
//!
//! - **Zeroization**: All key material uses `zeroize` to clear memory on drop
//! - **Constant-time comparison**: MAC verification uses `subtle::ConstantTimeEq`
//! - **Future-ready**: Abstractions for post-quantum cryptography (ML-KEM)
//!
//! # Example
//!
//! ```
//! use opentdf_crypto::tdf::TdfEncryption;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create encryption instance with generated keys
//! let tdf = TdfEncryption::new()?;
//!
//! // Encrypt data
//! let data = b"sensitive information";
//! let encrypted = tdf.encrypt(data)?;
//!
//! // Decrypt (legacy format)
//! let decrypted = TdfEncryption::decrypt_legacy(tdf.policy_key(), &encrypted)?;
//! assert_eq!(data, decrypted.as_slice());
//! # Ok(())
//! # }
//! ```

pub mod helpers;
pub mod hmac;
pub mod kem;
pub mod tdf;
pub mod types;

// Re-export commonly used types
pub use helpers::{CryptoError, create_aes_cipher, generate_key_32, generate_nonce};
pub use hmac::{
    HmacError, calculate_hmac, calculate_policy_binding, calculate_root_signature,
    verify_root_signature,
};
pub use kem::{KemError, KeyEncapsulation};
pub use tdf::{EncryptedPayload, SegmentInfo, SegmentedPayload, TdfEncryption};
pub use types::{AesKey, KeyError, Nonce96, PayloadKey, PolicyKey};

// NanoTDF exports (requires EC KEM)
#[cfg(feature = "kem-ec")]
pub use tdf::{
    NanoTdf, NanoTdfBuilder, NanoTdfCryptoError, NanoTdfError, NanoTdfIv, NanoTdfPayload,
    NanoTdfSignature, TagSize, decrypt, encrypt, generate_gmac, verify_gmac,
};

// RSA KEM exports
#[cfg(feature = "kem-rsa")]
pub use kem::rsa::{OaepHash, RsaOaepKem, wrap_key_with_rsa_oaep};

// EC KEM exports
#[cfg(feature = "kem-ec")]
pub use kem::ec::{EcCurve, EcWrappedKeyResult, EcdhKem, unwrap_key_with_ec, wrap_key_with_ec};

// PQC (Post-Quantum Cryptography) exports - always available
pub use kem::pqc::{HybridKem, MlKemLevel, MlKemWrapper};

#[cfg(feature = "kem-ec")]
pub use types::EcPrivateKey;

// Re-export underlying crypto libraries when KEM features are enabled
#[cfg(feature = "kem-ec")]
pub use hkdf;

#[cfg(feature = "kem-ec")]
pub use p256;

#[cfg(any(feature = "kem-rsa", feature = "kem-ec"))]
pub use pkcs8;

#[cfg(any(feature = "kem-rsa", feature = "kem-ec"))]
pub use sha1;

#[cfg(feature = "kem-ec")]
pub use sha2;

// Re-export rsa crate only for rustcrypto-provider feature (has RUSTSEC-2023-0071)
#[cfg(feature = "rustcrypto-provider")]
pub use rsa;

// Re-export encryption error
pub use tdf::encryption::EncryptionError;
