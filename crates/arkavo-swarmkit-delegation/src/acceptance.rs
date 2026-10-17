//! The checks a specialist makes before its binding (spec §7.3 step 3).

// Shared by the crate's modules only; plain `pub` here would trip
// `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::collections::HashMap;

use chrono::{DateTime, TimeDelta, Utc};

use crate::envelope::{Delegation, MAX_LIFETIME};
use crate::protocol::{Refusal, RefusalReason};

/// How far ahead of the specialist's clock `issued` may be.
pub(crate) const CLOCK_SKEW: TimeDelta = TimeDelta::minutes(5);

/// Nonces the specialist has accepted, each kept until the message it came
/// in can no longer be valid.
#[derive(Debug)]
pub struct NonceCache {
    kept: HashMap<(String, String), DateTime<Utc>>,
    capacity: usize,
}

impl NonceCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            kept: HashMap::new(),
            capacity,
        }
    }

    pub fn seen(&self, orchestrator: &str, nonce: &str, now: DateTime<Utc>) -> bool {
        self.kept
            .get(&(orchestrator.to_string(), nonce.to_string()))
            .is_some_and(|until| *until > now)
    }

    /// Keep `nonce` until `until`. Only messages from the bound orchestrator
    /// are recorded, so a full cache means that orchestrator sent this many
    /// live messages; the one closest to lapsing gives way.
    pub fn record(
        &mut self,
        orchestrator: &str,
        nonce: &str,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) {
        self.kept.retain(|_, kept_until| *kept_until > now);
        if self.kept.len() >= self.capacity
            && let Some(oldest) = self
                .kept
                .iter()
                .min_by_key(|(_, until)| **until)
                .map(|(key, _)| key.clone())
        {
            self.kept.remove(&oldest);
        }
        self.kept
            .insert((orchestrator.to_string(), nonce.to_string()), until);
    }
}

/// A nonce is 128 bits, base64url without padding (§7.2).
pub(crate) fn well_formed_nonce(nonce: &str) -> bool {
    nonce.len() == 22
        && nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Recipient, window and nonce (§7.3 step 3).
pub(crate) fn check_envelope(
    delegation: &Delegation,
    own_key: &str,
    nonces: &NonceCache,
    now: DateTime<Utc>,
) -> Result<(), Refusal> {
    if delegation.specialist != own_key {
        return Err(Refusal::new(
            RefusalReason::WrongSpecialist,
            format!("this specialist is {own_key}"),
        ));
    }
    if delegation.issued > now + CLOCK_SKEW {
        return Err(Refusal::new(
            RefusalReason::NotYetValid,
            format!("issued {} is ahead of this host's clock", delegation.issued),
        ));
    }
    if delegation.expires <= now {
        return Err(Refusal::new(
            RefusalReason::Expired,
            format!("expired at {}", delegation.expires),
        ));
    }
    if delegation.expires <= delegation.issued
        || delegation.expires - delegation.issued > MAX_LIFETIME
    {
        return Err(Refusal::new(
            RefusalReason::Expired,
            "the window must end after it starts and last at most 30 days",
        ));
    }
    if !well_formed_nonce(&delegation.nonce) {
        return Err(Refusal::new(
            RefusalReason::InvalidEnvelope,
            "nonce is not 128 bits of base64url",
        ));
    }
    if nonces.seen(&delegation.orchestrator, &delegation.nonce, now) {
        return Err(Refusal::new(
            RefusalReason::Replayed,
            "nonce already received",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_cache_lets_the_soonest_to_lapse_go() {
        let now = Utc::now();
        let mut cache = NonceCache::new(2);
        cache.record("o", "a", now + TimeDelta::minutes(1), now);
        cache.record("o", "b", now + TimeDelta::minutes(9), now);
        cache.record("o", "c", now + TimeDelta::minutes(5), now);
        assert!(!cache.seen("o", "a", now));
        assert!(cache.seen("o", "b", now));
        assert!(cache.seen("o", "c", now));
        assert!(!cache.seen("p", "b", now), "nonces are per orchestrator");
        assert!(!cache.seen("o", "b", now + TimeDelta::minutes(10)));
    }

    #[test]
    fn nonces_are_22_base64url_characters() {
        assert!(well_formed_nonce("AAAAAAAAAAAAAAAAAAAA-_"));
        assert!(!well_formed_nonce("AAAAAAAAAAAAAAAAAAAA+/"));
        assert!(!well_formed_nonce("short"));
    }
}
