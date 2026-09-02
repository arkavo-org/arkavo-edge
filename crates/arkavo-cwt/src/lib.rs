//! Verification of the short-lived agent CWT issued by authnz-rs.
//!
//! The token is a COSE_Sign1 (ES256) carrying CWT claims, transported as
//! base64url-without-padding. authnz-rs emits CBOR tag 61 (`D8 3D`) around an
//! otherwise untagged COSE_Sign1, but the parser is deliberately permissive
//! about the envelope: the tag-61 prefix is optional and the COSE_Sign1 may
//! also be tagged (tag 18), the form `arkavo-permit` mints. Signing keys are
//! published as a COSE key set at `<issuer>/.well-known/cose-keys`.
//!
//! Untrusted input over [`sign1::MAX_TOKEN_BYTES`] (16 KiB) is refused before
//! any CBOR work, as is input nesting deeper than
//! [`depth::MAX_NESTING_DEPTH`] (16) — including the CBOR carried inside the
//! COSE byte strings, which a decoder walks just as recursively.

pub mod claims;
pub mod depth;
pub mod key;
pub mod keys;
pub mod sign1;
pub mod verify;

pub use claims::Claims;
pub use key::VerifyingKey;
pub use keys::{CachedKeySet, KeySet};
pub use sign1::{ParsedSign1, parse};
pub use verify::{VerifyOptions, verify};

/// Every way verification can refuse a token.
#[derive(Debug, thiserror::Error)]
pub enum CwtError {
    #[error("token is not base64url without padding: {0}")]
    Base64(String),
    #[error("malformed COSE_Sign1: {0}")]
    Cose(String),
    #[error("malformed CWT claims: {0}")]
    Claims(String),
    #[error("unsupported signature algorithm: expected EdDSA or ES256, got {0}")]
    UnsupportedAlgorithm(String),
    #[error("COSE_Sign1 carries no kid header")]
    MissingKid,
    #[error("no published key with kid {0}")]
    UnknownKid(String),
    #[error("signature does not verify under the published key")]
    BadSignature,
    #[error("token expired at {exp} (now {now})")]
    Expired { exp: i64, now: i64 },
    #[error("token issued in the future: iat {iat} (now {now})")]
    IssuedInFuture { iat: i64, now: i64 },
    #[error("issuer mismatch: expected {expected}, got {actual}")]
    IssuerMismatch { expected: String, actual: String },
    #[error("audience {expected} is not in the token's aud")]
    AudienceMismatch { expected: String },
    #[error("malformed COSE key set: {0}")]
    KeySet(String),
    #[error("could not fetch the COSE key set: {0}")]
    Fetch(String),
    #[error("unusable COSE key: {0}")]
    Key(String),
    #[error("signature algorithm does not match the key type")]
    KeyAlgorithmMismatch,
}
