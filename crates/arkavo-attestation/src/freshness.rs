//! Attestation freshness / TTL expiry (HATT-009).
//!
//! An attestation quote is fresh relative to the verifier challenge it answers,
//! but a held attestation also carries a local TTL: once that TTL elapses (or
//! the orchestrator issues a new challenge) the agent must re-enter attestation
//! with a fresh nonce and produce a new quote. This module supplies only the
//! pure TTL-expiry predicate that drives that re-entry decision.
//!
//! Scope note: the resulting Trusted -> Suspended state transition on a
//! re-attestation *failure* is handled by the trusted-agent orchestration
//! lifecycle, not by this crate. This module decides only *when* a held
//! attestation has gone stale and must be refreshed.

/// Returns `true` when a held attestation issued at `issued_at_unix` with a
/// `ttl_secs` lifetime is expired as of `now_unix` and must be re-attested.
///
/// Expiry is reached at or after `ttl_secs` of age
/// (`now_unix - issued_at_unix >= ttl_secs`). The predicate is conservative on
/// clock skew: if the clock appears to have moved backwards
/// (`now_unix < issued_at_unix`), the attestation is treated as expired so the
/// agent fails closed and re-attests rather than trusting an un-anchored clock.
pub fn is_attestation_expired(issued_at_unix: i64, ttl_secs: i64, now_unix: i64) -> bool {
    if now_unix < issued_at_unix {
        // Clock moved backwards: not anchored. Fail closed and re-attest.
        return true;
    }
    // Saturate the age computation: on extreme inputs a plain `now - issued`
    // overflows i64 (panic in debug, wrap in release that could wrongly report
    // NOT expired and defeat fail-closed). Saturation pins the age at i64::MAX.
    now_unix.saturating_sub(issued_at_unix) >= ttl_secs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quote::{QuoteSigner, SoftwareQuoteSigner, produce_quote, verify_quote_envelope};
    use crate::verifier::{RejectReason, VerifyOutcome};
    use arkavo_test_macros::spec;

    fn fresh_nonce() -> Vec<u8> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .to_le_bytes()
            .to_vec()
    }

    #[spec("HATT-009")]
    #[test]
    fn ttl_expiry_boundary_is_inclusive_at_ttl_seconds() {
        // One second before the TTL elapses: still fresh.
        assert!(!is_attestation_expired(1000, 300, 1299));
        // Exactly at the TTL boundary: expired.
        assert!(is_attestation_expired(1000, 300, 1300));
        // Past the TTL boundary: expired.
        assert!(is_attestation_expired(1000, 300, 1301));
    }

    #[spec("HATT-009")]
    #[test]
    fn clock_moving_backwards_forces_re_attestation() {
        // now < issued_at: the clock is not anchored; fail closed and re-attest.
        assert!(is_attestation_expired(1000, 300, 999));
    }

    #[spec("HATT-009")]
    #[test]
    fn extreme_age_does_not_overflow_and_reports_expired() {
        // A held attestation issued at i64::MIN evaluated at i64::MAX has an age
        // far beyond any TTL. The naive `now - issued` subtraction overflows i64
        // (panic in debug, wrap in release that could wrongly report NOT
        // expired). The predicate must saturate and fail closed: expired.
        assert!(is_attestation_expired(i64::MIN, 300, i64::MAX));
    }

    #[spec("HATT-009")]
    #[test]
    fn re_attestation_produces_fresh_quote_that_stales_the_old_one() {
        let signer = SoftwareQuoteSigner::generate();
        let did = signer.did_key();

        // The original quote answered the first challenge.
        let old_nonce = fresh_nonce();
        let old = produce_quote(&signer, &old_nonce, b"m");
        // On TTL expiry / orchestrator challenge, the agent re-enters with a
        // fresh nonce and produces a new quote.
        let new_nonce = fresh_nonce();
        let new = produce_quote(&signer, &new_nonce, b"m");

        // The fresh quote satisfies the fresh challenge.
        assert_eq!(
            verify_quote_envelope(&new, &new_nonce, &did),
            VerifyOutcome::Accepted
        );
        // The stale quote cannot satisfy the fresh challenge nonce.
        assert_eq!(
            verify_quote_envelope(&old, &new_nonce, &did),
            VerifyOutcome::Rejected(RejectReason::NonceMismatch)
        );
    }
}
