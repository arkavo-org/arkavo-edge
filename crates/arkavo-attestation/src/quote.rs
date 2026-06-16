//! Signed attestation quotes over a verifier-supplied nonce (HATT-003).
//!
//! A quote is a signature over `{nonce, agent did:key, platform measurements}`.
//! Freshness derives from the verifier-supplied nonce: a quote is only fresh
//! relative to the challenge it answers, never to a local timestamp. This is the
//! full quote envelope that `verifier::verify_quote` (a nonce-only primitive)
//! does not yet cover.

use crate::verifier::{RejectReason, VerifyOutcome};

/// A signed attestation quote binding a nonce, the agent's did:key, and platform
/// measurements under the agent's attestation key.
#[derive(Debug, Clone)]
pub struct Quote {
    /// The verifier-supplied challenge this quote answers.
    pub nonce: Vec<u8>,
    /// The agent's P-256 did:key (identity bound into the signature).
    pub did_key: String,
    /// Opaque platform measurements covered by the signature.
    pub measurements: Vec<u8>,
    /// ECDSA P-256 (DER) signature over the canonical quote message.
    pub signature: Vec<u8>,
}

/// Canonical, unambiguous encoding of the quote contents.
///
/// Each field is length-prefixed with its u32 big-endian byte length so that no
/// two distinct field splits can produce the same byte string (e.g. a nonce of
/// `ab` + did `c` cannot collide with a nonce of `a` + did `bc`).
fn quote_message(nonce: &[u8], did_key: &str, measurements: &[u8]) -> Vec<u8> {
    let did_bytes = did_key.as_bytes();
    let mut message = Vec::with_capacity(12 + nonce.len() + did_bytes.len() + measurements.len());
    for field in [nonce, did_bytes, measurements] {
        debug_assert!(
            field.len() <= u32::MAX as usize,
            "quote field exceeds the u32 length prefix"
        );
        message.extend_from_slice(&(field.len() as u32).to_be_bytes());
        message.extend_from_slice(field);
    }
    message
}

/// Something that can sign a quote message and report the did:key it signs as.
///
/// Implementations MUST guarantee that [`QuoteSigner::did_key`] is the public
/// key corresponding to the private key used by [`QuoteSigner::sign`]; a quote's
/// identity binding rests on this invariant.
pub trait QuoteSigner {
    /// The signer's P-256 did:key.
    fn did_key(&self) -> String;
    /// Sign the canonical quote message, returning a DER ECDSA signature.
    fn sign(&self, message: &[u8]) -> Vec<u8>;
}

/// Software-backed quote signer (no hardware binding). Used where no Secure
/// Enclave / TPM key is available, and in tests.
pub struct SoftwareQuoteSigner {
    keypair: arkavo_crypto::P256SigningKeypair,
}

impl SoftwareQuoteSigner {
    /// Generate a fresh software signer with a new P-256 keypair.
    pub fn generate() -> Self {
        Self {
            keypair: arkavo_crypto::P256SigningKeypair::generate(),
        }
    }
}

impl QuoteSigner for SoftwareQuoteSigner {
    fn did_key(&self) -> String {
        self.keypair.public_key().to_did_key()
    }

    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.keypair.sign(message)
    }
}

/// Produce a signed quote over `nonce` and `measurements` using `signer`.
pub fn produce_quote(signer: &dyn QuoteSigner, nonce: &[u8], measurements: &[u8]) -> Quote {
    let did_key = signer.did_key();
    let signature = signer.sign(&quote_message(nonce, &did_key, measurements));
    Quote {
        nonce: nonce.to_vec(),
        did_key,
        measurements: measurements.to_vec(),
        signature,
    }
}

/// Verify a full quote envelope against the verifier's expected nonce and the
/// expected agent identity.
///
/// Acceptance means the quote is a fresh, valid quote *from the expected agent
/// identity* (`expected_did_key`): the nonce matches the challenge, the quote
/// claims the expected did:key, and the signature validates under the key
/// reconstructed from that did:key.
///
/// Rejects a stale/relayed quote (`NonceMismatch`) before any signature work,
/// then rejects a quote that does not claim the expected identity
/// (`DidMismatch`) — without this a valid self-signed quote from an unexpected
/// key over the right nonce would be accepted (identity confusion). Finally it
/// reconstructs the agent key from the embedded did:key and verifies the
/// signature over the canonical quote message.
pub fn verify_quote_envelope(
    quote: &Quote,
    expected_nonce: &[u8],
    expected_did_key: &str,
) -> VerifyOutcome {
    if quote.nonce != expected_nonce {
        return VerifyOutcome::Rejected(RejectReason::NonceMismatch);
    }

    if quote.did_key != expected_did_key {
        return VerifyOutcome::Rejected(RejectReason::DidMismatch);
    }

    let key = match arkavo_crypto::P256VerifyingKey::from_did_key(&quote.did_key) {
        Ok(key) => key,
        Err(_) => return VerifyOutcome::Rejected(RejectReason::BadSignature),
    };

    let message = quote_message(&quote.nonce, &quote.did_key, &quote.measurements);
    if key.verify(&message, &quote.signature).is_err() {
        return VerifyOutcome::Rejected(RejectReason::BadSignature);
    }

    VerifyOutcome::Accepted
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("HATT-003")]
    #[test]
    fn fresh_quote_over_nonce_and_measurements_is_accepted() {
        let signer = SoftwareQuoteSigner::generate();
        let nonce = b"verifier-challenge-001";
        let measurements = b"pcr0:deadbeef|pcr7:cafef00d";

        let quote = produce_quote(&signer, nonce, measurements);
        let did = signer.did_key();

        assert_eq!(quote.nonce, nonce);
        assert_eq!(quote.did_key, did);
        assert_eq!(
            verify_quote_envelope(&quote, nonce, &did),
            VerifyOutcome::Accepted
        );
    }

    #[spec("HATT-003")]
    #[test]
    fn tampered_signature_is_rejected_bad_signature() {
        let signer = SoftwareQuoteSigner::generate();
        let nonce = b"verifier-challenge-002";
        let measurements = b"pcr0:0011";

        let mut quote = produce_quote(&signer, nonce, measurements);
        let did = signer.did_key();
        let last = quote.signature.len() - 1;
        quote.signature[last] ^= 0xFF;

        assert_eq!(
            verify_quote_envelope(&quote, nonce, &did),
            VerifyOutcome::Rejected(RejectReason::BadSignature)
        );
    }

    #[spec("HATT-003")]
    #[test]
    fn mutated_measurements_after_signing_is_rejected_bad_signature() {
        let signer = SoftwareQuoteSigner::generate();
        let nonce = b"verifier-challenge-003";
        let measurements = b"pcr0:trusted";

        let mut quote = produce_quote(&signer, nonce, measurements);
        let did = signer.did_key();
        // Forge the measurements without re-signing: the signature no longer
        // covers them, so verification must fail.
        quote.measurements = b"pcr0:compromised".to_vec();

        assert_eq!(
            verify_quote_envelope(&quote, nonce, &did),
            VerifyOutcome::Rejected(RejectReason::BadSignature)
        );
    }

    #[spec("HATT-003")]
    #[test]
    fn stale_quote_against_different_nonce_is_rejected_nonce_mismatch() {
        let signer = SoftwareQuoteSigner::generate();
        let signed_nonce = b"verifier-challenge-004";
        let measurements = b"pcr0:fresh";

        let quote = produce_quote(&signer, signed_nonce, measurements);
        let did = signer.did_key();
        let different_nonce = b"a-newer-challenge-005";

        assert_eq!(
            verify_quote_envelope(&quote, different_nonce, &did),
            VerifyOutcome::Rejected(RejectReason::NonceMismatch)
        );
    }

    #[spec("HATT-007")]
    #[test]
    fn quote_from_unexpected_identity_is_rejected_did_mismatch() {
        // Signer A produces a perfectly valid, fresh self-signed quote over the
        // correct nonce. The verifier expects a DIFFERENT identity (signer B).
        // Without an identity binding, this valid self-signed quote would be
        // accepted (identity confusion); it must be rejected as DidMismatch.
        let signer_a = SoftwareQuoteSigner::generate();
        let signer_b = SoftwareQuoteSigner::generate();
        let nonce = b"verifier-challenge-006";
        let measurements = b"pcr0:from-a";

        let quote = produce_quote(&signer_a, nonce, measurements);
        let expected_did_b = signer_b.did_key();

        assert_eq!(
            verify_quote_envelope(&quote, nonce, &expected_did_b),
            VerifyOutcome::Rejected(RejectReason::DidMismatch)
        );
    }
}
