//! OpenTDF Protocol Types
//!
//! This crate contains protocol types and structures for OpenTDF, including:
//! - KAS (Key Access Service) request/response types
//! - TDF manifest structures
//! - Policy and attribute definitions
//! - Binary serialization for NanoTDF
//!
//! This crate contains NO cryptographic operations and NO I/O.
//! It is purely focused on data structures and serialization.

pub mod binary;
pub mod kas;
pub mod manifest;
pub mod nanotdf;

// Re-export commonly used types
pub use kas::{
    KasError, KasPolicyBinding, KeyAccessObject, KeyAccessObjectWrapper, KeyAccessRewrapResult,
    Policy as KasPolicy, PolicyRequest, PolicyRewrapResult, RewrapResponse, SignedRewrapRequest,
    UnsignedRewrapRequest, algorithm,
};

pub use manifest::{
    EncryptionInformation, EncryptionMethod, IntegrityInformation, KeyAccess, Payload,
    PolicyBinding, RootSignature, Segment, TdfManifest, key_access_type,
};
