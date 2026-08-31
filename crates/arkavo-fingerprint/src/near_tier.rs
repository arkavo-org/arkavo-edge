//! The near-duplicate tier as the cascade sees it (SENT-006).
//!
//! Second in the cascade, after the exact tier and before anything that runs a
//! model. Its judgement is document-level: a fingerprint is a majority vote
//! over every shingle in the span, so it cannot answer at all until it has seen
//! the whole span. That is why a budget overrun here reports *unavailable*
//! rather than a partial result — half a fingerprint is not an approximate
//! fingerprint, it is a different one.

use std::sync::Arc;
use std::time::{Duration, Instant};

use arkavo_protocol::classification_evidence::{Confidence, LabelFinding, TierReport};

use crate::key::IndexKey;
use crate::simhash::{MAX_HAMMING, MIN_SHINGLES, NearDuplicateIndex, NearMatch};

pub const NEAR_TIER_NAME: &str = "near-duplicate";

/// Share of the per-call budget this tier may spend when it runs alone.
///
/// Smaller than the exact tier's: this one runs second, on a budget the exact
/// tier has already drawn from, and it is the tier the cascade can most
/// afford to defer — the exact tier settles a verbatim copy without it.
pub const NEAR_TIER_BUDGET: Duration = Duration::from_micros(15);

/// Largest span fingerprinted inline.
///
/// The same limit the exact tier uses, not a smaller one: this tier needs *more*
/// text than that tier to say anything at all, so its real constraint is the
/// shared deadline, not a size of its own. Spans past this go to the
/// asynchronous path, which is where document-scale text arrives anyway.
const MAX_INLINE_BYTES: usize = 1024;

pub struct NearDuplicateTier {
    index: Option<Arc<NearDuplicateIndex>>,
    key: Option<Arc<IndexKey>>,
    unavailable_reason: String,
    budget: Duration,
}

impl NearDuplicateTier {
    pub fn unloaded(reason: impl Into<String>) -> Self {
        Self {
            index: None,
            key: None,
            unavailable_reason: reason.into(),
            budget: NEAR_TIER_BUDGET,
        }
    }

    pub fn loaded(index: Arc<NearDuplicateIndex>, key: Arc<IndexKey>) -> Self {
        if let Err(e) = index.check_key(&key) {
            return Self::unloaded(e.to_string());
        }
        Self {
            index: Some(index),
            key: Some(key),
            unavailable_reason: String::new(),
            budget: NEAR_TIER_BUDGET,
        }
    }

    #[must_use]
    pub fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    pub fn is_loaded(&self) -> bool {
        self.index.is_some()
    }

    pub fn version(&self) -> String {
        match &self.index {
            Some(index) => format!("{}+{}", index.format_version, index.taxonomy_version),
            None => "unloaded".to_string(),
        }
    }

    /// Examine a span against this tier's own budget.
    pub fn examine(&self, text: &str) -> TierReport {
        self.examine_until(text, Instant::now() + self.budget)
    }

    /// Examine a span, stopping at a deadline the cascade owns.
    ///
    /// The cascade passes one deadline through every tier so that the *sum* of
    /// the tiers stays inside the per-call budget, rather than each tier
    /// independently spending its own share of it.
    pub fn examine_until(&self, text: &str, deadline: Instant) -> TierReport {
        let (Some(index), Some(key)) = (&self.index, &self.key) else {
            return TierReport::unavailable(
                NEAR_TIER_NAME,
                self.version(),
                &self.unavailable_reason,
            );
        };
        if text.len() > MAX_INLINE_BYTES {
            return TierReport::unavailable(
                NEAR_TIER_NAME,
                self.version(),
                format!(
                    "span of {} bytes exceeds the {MAX_INLINE_BYTES}-byte inline limit; \
                     scoring deferred",
                    text.len()
                ),
            );
        }
        match crate::simhash::fingerprint_until(key, text, Some(deadline)) {
            Ok(found) => self.judge(index, found),
            Err(examined) => TierReport::unavailable(
                NEAR_TIER_NAME,
                self.version(),
                format!("budget exhausted after {examined} shingles; scoring deferred"),
            ),
        }
    }

    /// Examine a span with no deadline, for the asynchronous path a deferral
    /// hands the span to.
    pub fn examine_unbudgeted(&self, text: &str) -> TierReport {
        let (Some(index), Some(key)) = (&self.index, &self.key) else {
            return TierReport::unavailable(
                NEAR_TIER_NAME,
                self.version(),
                &self.unavailable_reason,
            );
        };
        match crate::simhash::fingerprint_until(key, text, None) {
            Ok(found) => self.judge(index, found),
            // Unreachable without a deadline, but reporting a gap is the safe
            // reading of an impossible state.
            Err(examined) => TierReport::unavailable(
                NEAR_TIER_NAME,
                self.version(),
                format!("fingerprinting abandoned after {examined} shingles"),
            ),
        }
    }

    /// Turn a fingerprint into a report, or say why there is none.
    ///
    /// A span too short to fingerprint reliably is reported as a gap, not as a
    /// clean miss. The distinction matters: this tier cannot see a short span,
    /// and letting that read as "nothing here" is how a cascade quietly stops
    /// covering the sizes it was never able to cover.
    fn judge(&self, index: &NearDuplicateIndex, found: Option<(u128, usize)>) -> TierReport {
        match found {
            Some((fingerprint, shingles)) if shingles >= MIN_SHINGLES => {
                self.report(index.nearest(fingerprint))
            }
            Some((_, shingles)) => TierReport::unavailable(
                NEAR_TIER_NAME,
                self.version(),
                format!(
                    "{shingles} shingles is below the {MIN_SHINGLES} a stable fingerprint                      needs; the exact tier covers this size"
                ),
            ),
            None => {
                TierReport::unavailable(NEAR_TIER_NAME, self.version(), "no words to fingerprint")
            }
        }
    }

    fn report(&self, found: Option<NearMatch>) -> TierReport {
        let findings = match found {
            Some(near) => vec![
                LabelFinding::new(
                    near.meta.category,
                    near.meta.sensitivity,
                    Confidence::new(near.confidence()),
                    format!("fingerprint within {} bits of {MAX_HAMMING}", near.distance),
                )
                .from_family(&near.meta.source_family),
            ],
            None => Vec::new(),
        };
        TierReport::matched(NEAR_TIER_NAME, self.version(), findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::classification_evidence::TierOutcome;
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
    use arkavo_test_macros::spec;

    use crate::index::EntryMeta;
    use crate::simhash::NearDuplicateIndex;

    fn document(seed: usize) -> String {
        use std::fmt::Write as _;
        (0..140).fold(String::new(), |mut text, i: usize| {
            let _ = write!(
                text,
                "t{}n{} ",
                (i * 37 + seed * 7919) % 991,
                (i * 13 + seed) % 577
            );
            text
        })
    }

    fn key() -> Arc<IndexKey> {
        Arc::new(IndexKey::derive(&[5u8; 32], "near-tier-tests").expect("derive"))
    }

    fn tier(key: &Arc<IndexKey>) -> NearDuplicateTier {
        let mut builder = NearDuplicateIndex::builder(key, "1.0.0");
        builder.add_document(
            key,
            &document(1),
            EntryMeta {
                category: DataCategory::Internal,
                sensitivity: SensitivityLevel::Confidential,
                source_family: "board".into(),
            },
        );
        NearDuplicateTier::loaded(Arc::new(builder.build()), key.clone())
    }

    /// SENT-002: a tier that contributed no signal is recorded as consulted
    /// with no match, never omitted.
    #[spec("SENT-002")]
    #[test]
    fn a_clean_miss_is_reported_as_a_match_with_no_findings() {
        let key = key();

        let report = tier(&key).examine_unbudgeted(&document(2));

        // NoMatch, not Unavailable: the tier saw the whole span and there was
        // no document in it. An outage would have to read differently.
        assert!(matches!(report.outcome, TierOutcome::NoMatch), "{report:?}");
        assert!(!report.is_unavailable());
    }

    /// SENT-006: evidence names which tier produced the label.
    #[spec("SENT-006")]
    #[test]
    fn a_near_duplicate_is_reported_with_its_family_and_tier() {
        let key = key();

        let report = tier(&key).examine_unbudgeted(&document(1));

        assert_eq!(report.tier, NEAR_TIER_NAME);
        let finding = report.findings().first().expect("a match");
        assert_eq!(finding.sensitivity, SensitivityLevel::Confidential);
        assert_eq!(finding.source_family.as_deref(), Some("board"));
    }

    /// SENT-013: an outage must not read as a clean result.
    #[spec("SENT-013")]
    #[test]
    fn an_unloaded_tier_reports_unavailable_rather_than_no_match() {
        let report = NearDuplicateTier::unloaded("no index provisioned").examine("anything at all");

        assert!(report.is_unavailable());
        assert!(report.findings().is_empty());
    }

    #[test]
    fn a_key_that_did_not_build_the_index_leaves_the_tier_unloaded() {
        // Matching nothing forever is worse than reporting a gap: one is
        // visible in the evidence and the other looks like a clean corpus.
        let key = key();
        let mut builder = NearDuplicateIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            &document(1),
            EntryMeta {
                category: DataCategory::Internal,
                sensitivity: SensitivityLevel::Confidential,
                source_family: "board".into(),
            },
        );
        let wrong = Arc::new(IndexKey::derive(&[6u8; 32], "near-tier-tests").expect("derive"));

        let tier = NearDuplicateTier::loaded(Arc::new(builder.build()), wrong);

        assert!(!tier.is_loaded());
        assert!(tier.examine(&document(1)).is_unavailable());
    }

    /// A span this tier cannot judge is a gap, not a clean miss.
    #[spec("SENT-006")]
    #[test]
    fn a_span_too_short_to_fingerprint_is_a_gap() {
        let key = key();

        let report = tier(&key).examine_unbudgeted("a short line of text");

        assert!(report.is_unavailable());
    }

    #[test]
    fn a_span_past_the_inline_limit_defers_rather_than_overrunning() {
        let key = key();
        let long = document(1).repeat(4);

        let report = tier(&key).examine(&long);

        assert!(
            report.is_unavailable(),
            "a large span must defer, not run inline"
        );
        // And the deferred path still finds it.
        assert!(!tier(&key).examine_unbudgeted(&long).findings().is_empty());
    }

    /// KP-011: a span that cannot be examined inside the deadline defers
    /// instead of stalling the call that is waiting on it.
    #[spec("KP-011")]
    #[test]
    fn an_exhausted_deadline_defers_rather_than_blocking() {
        let key = key();

        let report = tier(&key).examine_until(&document(1), Instant::now());

        assert!(report.is_unavailable());
    }
}
