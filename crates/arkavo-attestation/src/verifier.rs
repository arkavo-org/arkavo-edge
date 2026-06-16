//! Remote verification of signed attestation quotes (HATT-007).
//!
//! This slice implements the cryptographic core of remote verification: the
//! quote signature must validate over the supplied nonce under the attested
//! SEC1 public key, and the `claimed_did_key` must equal the did:key derived
//! from that same public key.
//!
//! Deferred (out of scope for this slice, tracked under HATT-007 /
//! github issue 628): full certificate-chain validation to a platform root,
//! nonce-freshness/replay checking, the complete quote envelope, and the
//! TLS-exporter channel binding that defeats a relayed-but-fresh quote.

/// Outcome of remote quote verification.
#[derive(Debug, PartialEq, Eq)]
pub enum VerifyOutcome {
    Accepted,
    Rejected(RejectReason),
}

/// Reason a quote was rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum RejectReason {
    BadSignature,
    DidMismatch,
    /// The quote's nonce did not match the verifier-supplied challenge, so the
    /// quote cannot be considered fresh (HATT-003).
    NonceMismatch,
}

/// Low-level primitive: verify a signature over a bare `nonce` under the SEC1
/// public key, AND that `claimed_did_key` equals the did:key derived from it.
///
/// This binds only the nonce. For full attestation-quote verification that also
/// binds the agent did:key and platform measurements (and checks nonce
/// freshness), use [`crate::quote::verify_quote_envelope`] — that is the
/// canonical entry point; this primitive is for callers needing only a nonce
/// challenge.
///
/// Malformed public-key bytes yield `Rejected(BadSignature)`.
pub fn verify_quote(
    public_key_sec1: &[u8],
    nonce: &[u8],
    signature: &[u8],
    claimed_did_key: &str,
) -> VerifyOutcome {
    // Malformed key material is indistinguishable from an unverifiable
    // signature for the purposes of this slice: reject as BadSignature.
    let key = match arkavo_crypto::P256VerifyingKey::from_sec1_bytes(public_key_sec1) {
        Ok(key) => key,
        Err(_) => return VerifyOutcome::Rejected(RejectReason::BadSignature),
    };

    if key.verify(nonce, signature).is_err() {
        return VerifyOutcome::Rejected(RejectReason::BadSignature);
    }

    if key.to_did_key() != claimed_did_key {
        return VerifyOutcome::Rejected(RejectReason::DidMismatch);
    }

    VerifyOutcome::Accepted
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn fresh_nonce() -> Vec<u8> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .to_le_bytes()
            .to_vec()
    }

    fn keypair() -> arkavo_crypto::P256SigningKeypair {
        arkavo_crypto::P256SigningKeypair::generate()
    }

    #[spec("HATT-007")]
    #[test]
    fn valid_signature_and_matching_did_is_accepted() {
        let kp = keypair();
        let nonce = fresh_nonce();
        let signature = kp.sign(&nonce);
        let public_key_sec1 = kp.public_key().to_sec1_bytes();
        let did = kp.public_key().to_did_key();

        let outcome = verify_quote(&public_key_sec1, &nonce, &signature, &did);
        assert_eq!(outcome, VerifyOutcome::Accepted);
    }

    #[spec("HATT-007")]
    #[test]
    fn signature_over_different_nonce_is_rejected_bad_signature() {
        let kp = keypair();
        let signed_nonce = fresh_nonce();
        let verifier_nonce = fresh_nonce();
        let signature = kp.sign(&signed_nonce);
        let public_key_sec1 = kp.public_key().to_sec1_bytes();
        let did = kp.public_key().to_did_key();

        let outcome = verify_quote(&public_key_sec1, &verifier_nonce, &signature, &did);
        assert_eq!(outcome, VerifyOutcome::Rejected(RejectReason::BadSignature));
    }

    #[spec("HATT-007")]
    #[test]
    fn tampered_signature_is_rejected_bad_signature() {
        let kp = keypair();
        let nonce = fresh_nonce();
        let mut signature = kp.sign(&nonce);
        // Flip a byte to corrupt the signature.
        let last = signature.len() - 1;
        signature[last] ^= 0xFF;
        let public_key_sec1 = kp.public_key().to_sec1_bytes();
        let did = kp.public_key().to_did_key();

        let outcome = verify_quote(&public_key_sec1, &nonce, &signature, &did);
        assert_eq!(outcome, VerifyOutcome::Rejected(RejectReason::BadSignature));
    }

    #[spec("HATT-007")]
    #[test]
    fn malformed_public_key_is_rejected_bad_signature() {
        let kp = keypair();
        let nonce = fresh_nonce();
        let signature = kp.sign(&nonce);
        let did = kp.public_key().to_did_key();
        let garbage = [0u8; 4];

        let outcome = verify_quote(&garbage, &nonce, &signature, &did);
        assert_eq!(outcome, VerifyOutcome::Rejected(RejectReason::BadSignature));
    }

    #[spec("HATT-007")]
    #[test]
    fn valid_signature_but_wrong_did_is_rejected_did_mismatch() {
        let kp = keypair();
        let nonce = fresh_nonce();
        let signature = kp.sign(&nonce);
        let public_key_sec1 = kp.public_key().to_sec1_bytes();
        // A did:key belonging to a different keypair.
        let wrong_did = keypair().public_key().to_did_key();

        let outcome = verify_quote(&public_key_sec1, &nonce, &signature, &wrong_did);
        assert_eq!(outcome, VerifyOutcome::Rejected(RejectReason::DidMismatch));
    }
}
