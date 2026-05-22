//! Agent Identity Authority (AIA).
//!
//! A dedicated role whose sole responsibility is to issue and attest agent
//! identity. An [`IdentityAuthority`] creates agent identities, issues signed
//! credentials, rotates agent keys, attests runtime/platform identity,
//! publishes a [`TrustChain`], and revokes compromised agents.
//!
//! The AIA deliberately does *not* assign work, grant resource access, make
//! ABAC decisions, delegate authority, or release TDF keys — those remain the
//! orchestrator's and KAS's responsibilities. An identity document yields only
//! `agent.identity.*` attributes; [`ORCHESTRATOR_ISSUED_ATTRIBUTES`] are the
//! authorization attributes that must never originate from identity.
//!
//! ```text
//! Root Identity Agent (trust anchor)
//!    └─ Swarm Identity Agent (issues identities)   ← IdentityAuthority
//!         └─ Orchestrator Agent (delegates work)   ← not part of this crate
//!              └─ Worker Agents
//! ```

mod authority;
mod credential;
mod document;
mod revocation;
mod trust_chain;

pub use authority::{AttestationEvidence, IdentityAuthority, IssueRequest};
pub use credential::IdentityCredential;
pub use document::{
    IdentityAttribute, IdentityDocument, ORCHESTRATOR_ISSUED_ATTRIBUTES, Runtime, TrustLevel,
};
pub use revocation::{RevocationClaims, RevocationList, RevokedEntry};
pub use trust_chain::{AuthorityEndorsement, EndorsementClaims, TrustChain};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use thiserror::Error;

/// Errors produced while issuing, verifying, or governing agent identity.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("signature verification failed")]
    InvalidSignature,
    #[error("credential or endorsement has expired")]
    Expired,
    #[error("authority '{0}' is not anchored to the trusted root")]
    UntrustedIssuer(String),
    #[error("runtime attestation reports a compromised platform")]
    CompromisedRuntime,
    #[error("malformed identity data: {0}")]
    Malformed(String),
    #[error(transparent)]
    Crypto(#[from] arkavo_crypto::CryptoError),
}

pub type Result<T> = std::result::Result<T, IdentityError>;

/// Deterministic bytes used as the signing input for a payload.
///
/// `serde_json` serializes structs in declaration order with no incidental
/// whitespace, so a parsed payload re-serializes to the exact bytes that were
/// signed.
fn canonical_bytes<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| IdentityError::Malformed(e.to_string()))
}

/// Sign a payload with an authority keypair, returning a base64 signature.
fn sign_payload<T: serde::Serialize>(
    keypair: &arkavo_crypto::AgentKeypair,
    payload: &T,
) -> Result<String> {
    let bytes = canonical_bytes(payload)?;
    Ok(STANDARD.encode(keypair.sign(&bytes)))
}

/// Verify a base64 signature over a payload against an authority public key.
fn verify_payload<T: serde::Serialize>(
    key: &arkavo_crypto::AgentPublicKey,
    payload: &T,
    signature_b64: &str,
) -> Result<()> {
    let bytes = canonical_bytes(payload)?;
    let signature = STANDARD
        .decode(signature_b64)
        .map_err(|e| IdentityError::Malformed(format!("signature is not base64: {e}")))?;
    key.verify(&bytes, &signature)
        .map_err(|_| IdentityError::InvalidSignature)
}
