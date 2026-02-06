//! OpenTDF KAS (Key Access Service) Server Crypto Operations
//!
//! This crate provides server-side cryptographic operations for KAS implementations:
//! - EC P-256 keypair management for NanoTDF
//! - RSA-2048 keypair management for Standard TDF
//! - ECDH key agreement and rewrap for NanoTDF
//! - RSA-OAEP unwrap and rewrap for Standard TDF
//! - NanoTDF version detection and salt computation
//!
//! # Features
//!
//! - `ec` (default): EC P-256 support for NanoTDF
//! - `rsa`: RSA-2048 support for Standard TDF
//!
//! # Example
//!
//! ```
//! use opentdf_kas::{KasEcKeypair, ec_unwrap};
//!
//! # fn example() -> Result<(), opentdf_kas::KasServerError> {
//! // Generate KAS EC keypair
//! let kas_keypair = KasEcKeypair::generate()?;
//!
//! // Parse client public key from PEM
//! let client_pem = kas_keypair.public_key_pem(); // In practice, from client
//! let _client_public = KasEcKeypair::parse_public_key_pem(client_pem)?;
//!
//! // EC unwrap would be called with NanoTDF header and ephemeral key
//! // let rewrapped = ec_unwrap(header, ephemeral_key, kas_keypair.private_key(), session_secret)?;
//! # Ok(())
//! # }
//! ```

#![allow(deprecated)] // For aes_gcm generic_array 0.x

pub mod error;
pub mod keypair;
pub mod rewrap;
pub mod salt;
pub mod types;
pub mod unwrap;

// Re-export error types
pub use error::KasServerError;

// Re-export keypair types
#[cfg(feature = "rsa")]
pub use keypair::KasRsaKeypair;
pub use keypair::{KasEcKeypair, KasKeypair};

// Re-export rewrap functions
pub use rewrap::{encode_wrapped_key, rewrap_dek, rewrap_dek_simple};

// Re-export salt functions
pub use salt::{
    NANOTDF_MAGIC, NanoTdfVersion, compute_nanotdf_salt, compute_nanotdf_salt_from_byte,
    detect_nanotdf_version,
};

// Re-export types
pub use types::{KeyAccessObject, KeyAlgorithm, KeyInfo, RewrapResult};

// Re-export unwrap functions
#[cfg(feature = "rsa")]
pub use unwrap::rsa_unwrap;
pub use unwrap::{custom_ecdh, ec_unwrap};

// Re-export underlying crypto libraries for advanced use cases
pub use p256;
#[cfg(feature = "rsa")]
pub use rsa;
