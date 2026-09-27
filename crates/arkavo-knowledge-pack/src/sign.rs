//! Signing and verifying the manifest (KP-002, KP-003).
//!
//! The signature covers the manifest **bytes as written**, not a re-serialized
//! copy of a parsed struct. Canonical-JSON round-tripping is where signature
//! schemes quietly break: a field reordered by a serde version bump, a float
//! rendered differently, an escape normalized, and a valid signature stops
//! verifying — or worse, two different documents verify against one signature.
//! Verification therefore reads the file, checks the signature over exactly
//! those bytes, and only then parses.

use arkavo_crypto::{AgentKeypair, AgentPublicKey};

#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("the manifest signature does not verify against the supplied anchor")]
    Invalid,
    #[error("no organization signing anchor was supplied; a pack cannot be trusted without one")]
    NoAnchor,
    #[error("the signature file is malformed: {0}")]
    Malformed(String),
}

/// Sign manifest bytes with the organization key.
pub fn sign_manifest(manifest_bytes: &[u8], key: &AgentKeypair) -> Vec<u8> {
    key.sign(manifest_bytes)
}

/// Verify a detached signature over manifest bytes.
///
/// Takes a *resolved* anchor public key. Resolving the organization's
/// `did:webvh` to that key is deliberately not done here: there is no resolver
/// in this workspace yet, and inventing one inside a verification routine would
/// make the trust root whatever the verifier felt like fetching. `None` is
/// refused rather than treated as "verify later" (KP-003: no trust on first
/// use).
pub fn verify_manifest(
    manifest_bytes: &[u8],
    signature: &[u8],
    anchor: Option<&AgentPublicKey>,
) -> Result<(), SignatureError> {
    let anchor = anchor.ok_or(SignatureError::NoAnchor)?;
    anchor
        .verify(manifest_bytes, signature)
        .map_err(|_| SignatureError::Invalid)
}

/// The signature file's contents: base64, one line, no framing.
pub fn encode_signature(signature: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "{}\n",
        base64::engine::general_purpose::STANDARD.encode(signature)
    )
}

pub fn decode_signature(text: &str) -> Result<Vec<u8>, SignatureError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|e| SignatureError::Malformed(e.to_string()))
}
