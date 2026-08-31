//! The probe budget shared by the gate and the sentinel (SENT-010).
//!
//! A classifier is an oracle. Send a payload, see whether it is refused, and
//! each answer is one bit about what the corpus contains; enough bits and the
//! index has been read out through its own denials. Rate limiting is what makes
//! that expensive.
//!
//! One bucket, not two. Separate budgets would let a caller exhaust the gate
//! and keep probing the sentinel, or alternate between them for twice the
//! answers — so the bucket is shared, and the throttle is keyed by **identity**
//! rather than by session, because opening a new session is free.

use std::sync::Arc;

use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;

/// Identity a probe is charged to.
pub type ProbeIdentity = String;

/// Identity recorded when a caller cannot be identified.
///
/// Everything anonymous shares one bucket rather than getting a free one each:
/// an unidentified caller must not be cheaper to be than an identified one.
pub const ANONYMOUS: &str = "anonymous";

type KeyedLimiter = RateLimiter<ProbeIdentity, DefaultKeyedStateStore<ProbeIdentity>, DefaultClock>;

/// A throttling decision. Deliberately not an error type: the caller renders a
/// generic denial from it, never a distinguishable rate-limit signal (SENT-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeVerdict {
    Within,
    Throttled,
}

impl ProbeVerdict {
    pub fn is_throttled(self) -> bool {
        matches!(self, ProbeVerdict::Throttled)
    }
}

/// A token bucket per identity, shared between every consumer holding a clone.
#[derive(Clone)]
pub struct ProbeBudget {
    limiter: Arc<KeyedLimiter>,
}

impl ProbeBudget {
    /// # Panics
    ///
    /// If `per_second` or `burst` is zero. A budget that permits nothing is a
    /// misconfiguration that would silently deny every call, so it fails at
    /// construction where it is visible.
    pub fn new(per_second: u32, burst: u32) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(per_second).expect("per_second must be > 0"))
            .allow_burst(NonZeroU32::new(burst).expect("burst must be > 0"));
        Self {
            limiter: Arc::new(RateLimiter::keyed(quota)),
        }
    }

    /// Charge one probe to an identity.
    pub fn charge(&self, identity: Option<&str>) -> ProbeVerdict {
        let key = identity.unwrap_or(ANONYMOUS).to_string();
        match self.limiter.check_key(&key) {
            Ok(()) => ProbeVerdict::Within,
            Err(_) => ProbeVerdict::Throttled,
        }
    }

    /// Drop per-identity state that has fully recovered, so a long-lived
    /// process does not accumulate a bucket per identity it has ever seen.
    pub fn shrink(&self) {
        self.limiter.retain_recent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// SENT-010: probing the gate throttles the sentinel too, because both hold
    /// a clone of the same bucket.
    #[spec("SENT-010")]
    #[test]
    fn the_gate_and_the_sentinel_draw_from_one_bucket() {
        let gate = ProbeBudget::new(1, 2);
        let sentinel = gate.clone();

        assert_eq!(gate.charge(Some("did:example:probe")), ProbeVerdict::Within);
        assert_eq!(gate.charge(Some("did:example:probe")), ProbeVerdict::Within);

        assert_eq!(
            sentinel.charge(Some("did:example:probe")),
            ProbeVerdict::Throttled,
            "the sentinel must see the budget the gate already spent"
        );
    }

    /// SENT-010 edge case: the budget is per identity, not per session, so
    /// opening a new session does not buy more probes.
    #[spec("SENT-010")]
    #[test]
    fn a_new_session_of_the_same_identity_gets_no_new_budget() {
        let budget = ProbeBudget::new(1, 1);
        let session_one = budget.clone();
        let session_two = budget;

        assert_eq!(
            session_one.charge(Some("did:example:a")),
            ProbeVerdict::Within
        );

        assert!(session_two.charge(Some("did:example:a")).is_throttled());
    }

    #[test]
    fn a_different_identity_has_its_own_budget() {
        // Otherwise one noisy caller denies service to everyone else.
        let budget = ProbeBudget::new(1, 1);
        budget.charge(Some("did:example:a"));

        assert_eq!(budget.charge(Some("did:example:b")), ProbeVerdict::Within);
    }

    #[test]
    fn unidentified_callers_share_one_budget() {
        // Being anonymous must not be cheaper than being identified.
        let budget = ProbeBudget::new(1, 1);

        assert_eq!(budget.charge(None), ProbeVerdict::Within);
        assert!(budget.charge(None).is_throttled());
    }
}
