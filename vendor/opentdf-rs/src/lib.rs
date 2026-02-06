//! OpenTDF - Trusted Data Format for Rust
//!
//! This crate provides a Rust implementation of the OpenTDF (Trusted Data Format) standard,
//! supporting:
//! - Standard TDF (TDF3, ZTDF) with segment-based encryption
//! - Attribute-Based Access Control (ABAC) policies
//! - Key Access Service (KAS) integration
//! - JSON-RPC format (ZTDF-JSON)
//! - Security hardening (zeroizing keys, constant-time verification)
//!
//! # Architecture
//!
//! - **`opentdf-protocol`**: Protocol types and structures (no crypto)
//! - **`opentdf-crypto`**: Cryptographic operations with security hardening
//! - **`opentdf`**: High-level TDF API and integration
//!
//! # Getting Started
//!
//! For the best experience, import the prelude to get all commonly used types and traits:
//!
//! ```no_run
//! use opentdf::prelude::*;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a policy with the new builder
//! let policy = PolicyBuilder::new()
//!     .id_auto()
//!     .dissemination(["user@example.com"])
//!     .build()?;
//!
//! // Encrypt data to a TDF file
//! Tdf::encrypt(b"sensitive data")
//!     .kas_url("https://kas.example.com")
//!     .policy(policy.clone())
//!     .to_file("output.tdf")?;
//!
//! // Extension trait methods work automatically with prelude!
//! let mut manifest = TdfManifest::new("payload".to_string(), "https://kas.example.com".to_string());
//! manifest.set_policy(&policy)?; // No need to import TdfManifestExt
//! # Ok(())
//! # }
//! ```

mod archive;
mod error;
pub mod fqn;
pub mod manifest;
mod policy;
mod tdf;

#[cfg(feature = "kas-client")]
pub mod kas;

#[cfg(feature = "kas-client")]
pub mod kas_key;

// JSON-RPC integration (ZTDF-JSON format)
pub mod jsonrpc;

// TDF-CBOR format (optional, requires "cbor" feature)
#[cfg(feature = "cbor")]
pub mod tdf_cbor;

// Prelude for convenient imports
pub mod prelude;

// Re-export protocol types
pub use opentdf_protocol::{
    EncryptionInformation, EncryptionMethod, IntegrityInformation, KeyAccess, Payload,
    PolicyBinding, RootSignature, Segment, TdfManifest,
};

// Re-export crypto types
pub use opentdf_crypto::{
    EncryptedPayload, EncryptionError, SegmentInfo, SegmentedPayload, TdfEncryption,
};

// Re-export KAS types (from protocol)
pub use opentdf_protocol::{
    KasError, KasPolicyBinding, KeyAccessObject, KeyAccessObjectWrapper, KeyAccessRewrapResult,
    PolicyRequest, PolicyRewrapResult, RewrapResponse, SignedRewrapRequest, UnsignedRewrapRequest,
};

// Re-export security types
pub use opentdf_crypto::{AesKey, KeyError, PayloadKey, PolicyKey};

// Re-export crypto libraries for KAS feature
#[cfg(feature = "kas-client")]
pub use opentdf_crypto::{hkdf, p256, pkcs8, sha1, sha2};

// Re-export rsa crate only for rustcrypto-rsa feature (legacy, has RUSTSEC-2023-0071)
#[cfg(feature = "rustcrypto-rsa")]
pub use opentdf_crypto::rsa;

// High-level API (primary interface)
pub use tdf::{Tdf, TdfEncryptBuilder, TdfEncryptFileBuilder};

#[cfg(feature = "kas-client")]
pub use tdf::{TdfDecryptBuilder, TdfDecryptFileBuilder};

// Core types
pub use archive::{TdfArchive, TdfArchiveBuilder, TdfArchiveMemoryBuilder, TdfError};

// Unified error type for SDK consumers
pub use error::OpenTdfError;

// Policy types
pub use policy::{
    AttributeCondition, AttributeIdentifier, AttributePolicy, AttributeValue, FqnError,
    FqnErrorKind, LogicalOperator, Operator, Policy, PolicyBody, PolicyBuilder, PolicyError,
    ValidationError, ValidationErrorType,
};

// FQN types
pub use fqn::{AttributeFqn, FqnValidationRules, NamespaceRegistry};

// KAS feature types
#[cfg(feature = "kas-client")]
pub use kas::{KasClient, KeyType};

#[cfg(feature = "kas-client")]
pub use kas_key::{
    KasEcPublicKeyResponse, KasKeyError, KasPublicKeyResponse, fetch_kas_ec_public_key,
    fetch_kas_public_key, validate_ec_public_key_pem, validate_rsa_public_key_pem,
};

#[cfg(feature = "kas-client")]
pub use opentdf_crypto::{EcdhKem, OaepHash, RsaOaepKem, wrap_key_with_rsa_oaep};

// TDF-JSON types (spec-compliant)
pub use jsonrpc::{JsonPayload, TdfJson, TdfJsonManifest};

#[cfg(feature = "kas-client")]
pub use jsonrpc::TdfJsonBuilder;

// Legacy JSON-RPC types (deprecated)
#[allow(deprecated)]
pub use jsonrpc::{InlinePayload, TdfJsonRpc, TdfJsonRpcBuilder, TdfManifestInline};

// NanoTDF Collection types (for streaming/RTMP use cases)
#[cfg(feature = "kas-client")]
pub use opentdf_crypto::tdf::{
    NanoTdfCollection, NanoTdfCollectionBuilder, NanoTdfCollectionDecryptor,
};

#[cfg(feature = "kas-client")]
pub use opentdf_protocol::nanotdf::CollectionItem;
