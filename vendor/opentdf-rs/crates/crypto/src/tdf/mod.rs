//! TDF encryption implementations
//!
//! This module contains encryption implementations for:
//! - Standard TDF (TDF3, ZTDF) - segment-based AES-256-GCM
//! - NanoTDF - compact format with EC key wrapping (requires `kem-ec` feature)
//! - NanoTDF Collection - multiple payloads with shared DEK (requires `kem-ec` feature)
//!
//! ## NanoTDF Crypto Backend
//! - Default: Mbed TLS (supports 64-128 bit GCM tags, 64-bit is NanoTDF default)
//! - Without `nanotdf-mbedtls`: RustCrypto (pure Rust, supports 96-128 bit tags only)

pub mod encryption;

// NanoTDF requires EC KEM which is only available with the `kem-ec` feature
#[cfg(feature = "kem-ec")]
pub mod nanotdf;

#[cfg(feature = "kem-ec")]
pub mod nanotdf_collection;

// Conditional backend selection for NanoTDF crypto
#[cfg(all(feature = "kem-ec", feature = "nanotdf-mbedtls"))]
pub mod nanotdf_crypto_mbedtls;
#[cfg(all(feature = "kem-ec", feature = "nanotdf-mbedtls"))]
pub use nanotdf_crypto_mbedtls as nanotdf_crypto;

#[cfg(all(feature = "kem-ec", not(feature = "nanotdf-mbedtls")))]
pub mod nanotdf_crypto;

pub use encryption::{EncryptedPayload, SegmentInfo, SegmentedPayload, TdfEncryption};
#[cfg(feature = "kem-ec")]
pub use nanotdf::{NanoTdf, NanoTdfBuilder, NanoTdfError, NanoTdfPayload, NanoTdfSignature};
#[cfg(feature = "kem-ec")]
pub use nanotdf_collection::{
    NanoTdfCollection, NanoTdfCollectionBuilder, NanoTdfCollectionDecryptor,
};
#[cfg(feature = "kem-ec")]
pub use nanotdf_crypto::{
    NanoTdfCryptoError, NanoTdfIv, TagSize, decrypt, encrypt, generate_gmac, verify_gmac,
};
