//! Core TDF (Trusted Data Format) abstraction layer for Arkavo.
//!
//! This crate provides streaming-first traits and types for TDF operations,
//! designed to handle GB-scale data without OOM issues.
//!
//! # Architecture
//!
//! - **Traits**: Abstract interfaces for encryption, decryption, and transport
//! - **Types**: ZTDF-JSON format for inline payloads in JSON-RPC
//! - **Decoupled Design**: BlobTransport handles raw bytes; encryption is composed at application layer
//!
//! # Example
//!
//! ```ignore
//! use arkavo_tdf::{TdfEncryptor, BlobTransport, PolicyBuilder};
//!
//! // Compose encryption + transport at application layer
//! async fn secure_stage<E, T>(
//!     encryptor: &E,
//!     transport: &T,
//!     data: &[u8],
//!     policy: &Policy,
//! ) -> Result<String, Box<dyn std::error::Error>>
//! where
//!     E: TdfEncryptor,
//!     T: BlobTransport,
//! {
//!     let manifest = encryptor.encrypt(data, policy).await?;
//!     let ticket = transport.stage(&manifest.payload.value.as_bytes()).await?;
//!     Ok(ticket)
//! }
//! ```

mod error;
mod policy;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod traits;
mod types;

// Real opentdf-rs integration (optional)
#[cfg(feature = "opentdf")]
pub mod opentdf_impl;

// KAS client with Arkavo OAuth integration (optional)
#[cfg(feature = "kas")]
pub mod kas_client;

pub use error::{TdfError, TransportError};
pub use policy::{PolicyBuilder, arkavo_attrs};
pub use traits::{BlobTransport, KasClient, TdfDecryptor, TdfEncryptor, TdfService};
pub use types::{
    Attribute, EncryptionInformation, EncryptionMethod, InlinePayload, KeyAccessObject, Policy,
    PolicyBinding, TdfManifest,
};

// Re-export opentdf implementation when feature is enabled
#[cfg(feature = "opentdf")]
pub use opentdf_impl::{OpenTdfConfig, OpenTdfService};

// Re-export KAS client when feature is enabled
#[cfg(feature = "kas")]
pub use kas_client::{
    ARKAVO_KAS_URL, ARKAVO_OAUTH_URL, ArkavoKasClient, ArkavoKasConfig, KasPublicKey,
};
