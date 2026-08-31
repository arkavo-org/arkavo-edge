//! The reference tier as the cascade sees it (KP-011, SENT-002).
//!
//! This is the fast tier: a keyed hash per shingle and a hash-map probe, sitting
//! ahead of anything that runs a model. It reports evidence — what matched, how
//! much of the span, which family — and decides nothing.
//!
//! Two behaviours matter more than speed. An index that is not loaded reports
//! *unavailable* rather than *no match*, because an outage that reads as a clean
//! result is how a cascade silently stops working. And a budget overrun stops
//! examining rather than blocking the call (KP-011): the tier is on the
//! per-call path, and a tier that can exceed the budget is a tier that can be
//! made to stall every tool call by feeding it a large span.

use std::sync::Arc;
use std::time::{Duration, Instant};

use arkavo_protocol::classification_evidence::{
    ClassificationEvidence, Confidence, LabelFinding, TierReport,
};

use crate::index::{MatchSummary, ReferenceIndex, match_span};
use crate::key::IndexKey;

pub const TIER_NAME: &str = "reference-index";

/// Share of the 50µs per-call budget this tier may spend.
///
/// Half, not all: the tier runs inside `ingest`, which also runs the pattern
/// detector, and the budget is shared across everything on that path.
pub const TIER_BUDGET: Duration = Duration::from_micros(25);

/// Shingles examined between clock reads.
///
/// Reading the clock per shingle would cost more than the hashing it guards.
const BUDGET_CHECK_STRIDE: usize = 16;

/// Largest span examined inline.
///
/// Measured at ~22ns per byte to normalize, shingle, hash and probe, so 25µs
/// buys about a kilobyte. The size test comes first and costs nothing: a
/// deferral decision that requires shingling the span has already spent what it
/// was trying to save, which is how the earlier version took 1.6ms to decide
/// not to spend 25µs.
const MAX_INLINE_BYTES: usize = 1024;

/// The reference tier, holding a loaded index or the reason it has none.
pub struct ReferenceTier {
    index: Option<Arc<ReferenceIndex>>,
    key: Option<Arc<IndexKey>>,
    unavailable_reason: String,
    budget: Duration,
}

impl ReferenceTier {
    /// A tier with no index. Reports unavailable; the cascade proceeds without it.
    pub fn unloaded(reason: impl Into<String>) -> Self {
        Self {
            index: None,
            key: None,
            unavailable_reason: reason.into(),
            budget: TIER_BUDGET,
        }
    }

    /// A tier backed by an index. The key must be the one that built it, or the
    /// tier stays unloaded rather than matching nothing forever.
    pub fn loaded(index: Arc<ReferenceIndex>, key: Arc<IndexKey>) -> Self {
        if let Err(e) = index.check_key(&key) {
            return Self::unloaded(e.to_string());
        }
        Self {
            index: Some(index),
            key: Some(key),
            unavailable_reason: String::new(),
            budget: TIER_BUDGET,
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

    /// Examine a span and report what this tier saw.
    pub fn examine(&self, text: &str) -> TierReport {
        let (Some(index), Some(key)) = (&self.index, &self.key) else {
            return TierReport::unavailable(TIER_NAME, self.version(), &self.unavailable_reason);
        };

        if text.len() > MAX_INLINE_BYTES {
            return TierReport::unavailable(
                TIER_NAME,
                self.version(),
                format!(
                    "span of {} bytes exceeds the {MAX_INLINE_BYTES}-byte inline limit; \
                     scoring deferred",
                    text.len()
                ),
            );
        }

        let started = Instant::now();
        let summary = self.match_within_budget(index, key, text, started);
        match summary {
            Ok(summary) => TierReport::matched(TIER_NAME, self.version(), findings(&summary)),
            // Not a match and not a clean miss: the span was only partly seen,
            // so the honest report is that this tier could not finish.
            Err(examined) => TierReport::unavailable(
                TIER_NAME,
                self.version(),
                format!(
                    "budget of {}µs exhausted after {examined} shingles; scoring deferred",
                    self.budget.as_micros()
                ),
            ),
        }
    }

    /// Examine a span, ignoring the budget. For the asynchronous tier that a
    /// budget overrun defers to, where wall-clock cost is no longer on the
    /// caller's path.
    pub fn examine_unbudgeted(&self, text: &str) -> TierReport {
        let (Some(index), Some(key)) = (&self.index, &self.key) else {
            return TierReport::unavailable(TIER_NAME, self.version(), &self.unavailable_reason);
        };
        TierReport::matched(
            TIER_NAME,
            self.version(),
            findings(&match_span(index, key, text)),
        )
    }

    fn match_within_budget(
        &self,
        index: &ReferenceIndex,
        key: &IndexKey,
        text: &str,
        started: Instant,
    ) -> Result<MatchSummary, usize> {
        // Normalized once; windows are joined one at a time so the budget can
        // stop the work rather than arriving after it.
        let normalized = crate::shingle::normalize(text);
        let words: Vec<&str> = normalized.split(' ').filter(|w| !w.is_empty()).collect();
        let mut summary = MatchSummary::default();
        for (n, shingle) in crate::shingle::windows(&words).enumerate() {
            if n % BUDGET_CHECK_STRIDE == 0 && n > 0 && started.elapsed() > self.budget {
                return Err(n);
            }
            summary.shingles_examined += 1;
            let hash = key.hash(&shingle);
            if index.suppression().contains(hash) {
                continue;
            }
            let Some(meta) = index.lookup(hash) else {
                continue;
            };
            summary.shingles_matched += 1;
            match summary.by_family.get(&meta.source_family) {
                Some(existing) if existing.sensitivity >= meta.sensitivity => {}
                _ => {
                    summary
                        .by_family
                        .insert(meta.source_family.clone(), meta.clone());
                }
            }
        }
        Ok(summary)
    }
}

/// Turn a match summary into evidence.
///
/// Confidence is coverage: the fraction of the span this index recognized. One
/// shingle in a long document is a coincidence; the same shingle in a two-line
/// span is the document. Reporting the raw match count instead would make a
/// threshold meaningless across span sizes.
fn findings(summary: &MatchSummary) -> Vec<LabelFinding> {
    if summary.is_empty() {
        return Vec::new();
    }
    let confidence = Confidence::new(summary.coverage());
    summary
        .by_family
        .iter()
        .map(|(family, meta)| {
            LabelFinding::new(
                meta.category,
                meta.sensitivity,
                confidence,
                format!(
                    "{}/{} shingles matched",
                    summary.shingles_matched, summary.shingles_examined
                ),
            )
            .from_family(family)
        })
        .collect()
}

/// Build evidence from a single tier. The cascade in Phase 4 will chain more.
pub fn evidence_for(
    tier: &ReferenceTier,
    taxonomy_version: &str,
    text: &str,
) -> ClassificationEvidence {
    ClassificationEvidence::new(taxonomy_version).with_tier(tier.examine(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::classification_evidence::TierOutcome;
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
    use arkavo_test_macros::spec;

    const CLASSIFIED: &str =
        "the acquisition of northwind holdings closes in the third quarter pending approval";

    fn tier() -> ReferenceTier {
        let key = Arc::new(IndexKey::derive(&[7u8; 32], "corpus").expect("derive"));
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board-minutes",
        );
        ReferenceTier::loaded(Arc::new(builder.build()), key)
    }

    #[spec("KP-011")]
    #[test]
    fn an_unloaded_index_reports_unavailable_not_no_match() {
        // A cascade that reads an outage as "nothing found" stops working
        // without anyone noticing.
        let tier = ReferenceTier::unloaded("index not yet fetched");

        let report = tier.examine(CLASSIFIED);

        assert!(report.is_unavailable());
        assert!(matches!(report.outcome, TierOutcome::Unavailable { .. }));
    }

    #[spec("KP-011")]
    #[test]
    fn a_wrong_key_leaves_the_tier_unloaded_rather_than_silently_empty() {
        let key = Arc::new(IndexKey::derive(&[7u8; 32], "corpus").expect("derive"));
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "f",
        );
        let wrong = Arc::new(IndexKey::derive(&[8u8; 32], "corpus").expect("derive"));

        let tier = ReferenceTier::loaded(Arc::new(builder.build()), wrong);

        assert!(!tier.is_loaded());
        assert!(tier.examine(CLASSIFIED).is_unavailable());
    }

    #[spec("KP-011")]
    #[test]
    fn a_clean_miss_is_a_match_report_with_no_findings() {
        let report = tier().examine("entirely unrelated prose about the weather this week");

        assert!(!report.is_unavailable());
        assert_eq!(report.outcome, TierOutcome::NoMatch);
    }

    #[spec("SENT-002")]
    #[test]
    fn a_hit_carries_confidence_family_and_version() {
        let tier = tier();

        let report = tier.examine(CLASSIFIED);
        let finding = &report.findings()[0];

        assert_eq!(finding.source_family.as_deref(), Some("board-minutes"));
        assert_eq!(finding.sensitivity, SensitivityLevel::Confidential);
        assert!(finding.confidence.value() > 0.0);
        assert_eq!(report.version, "1+1.0.0");
        assert!(finding.signal.contains("shingles matched"));
    }

    #[spec("KP-011")]
    #[test]
    fn a_budget_overrun_defers_rather_than_blocking() {
        // A tier that cannot exceed its budget is a tier an attacker cannot use
        // to stall every tool call by sending one enormous span.
        let tier = tier().with_budget(Duration::from_nanos(1));
        let long = std::iter::repeat_n(CLASSIFIED, 400)
            .collect::<Vec<_>>()
            .join(" ");

        let report = tier.examine(&long);

        assert!(report.is_unavailable(), "budget was not enforced");
        // ...and the deferred path still produces the finding.
        assert!(!tier.examine_unbudgeted(&long).findings().is_empty());
    }

    #[spec("KP-011")]
    #[test]
    fn a_short_span_is_answered_synchronously_inside_the_budget() {
        // Tool-call arguments — what the egress gate actually hands this tier —
        // are short, and that is the case the budget has to cover.
        let tier = tier();

        let report = tier.examine(CLASSIFIED);

        // Behaviour, not wall clock: a debug build measures unoptimized
        // shingling, so the budget number comes from the bench. What this pins
        // is that a short span is answered rather than deferred.
        assert!(!report.is_unavailable(), "a short span was deferred");
        assert!(!report.findings().is_empty());
    }

    #[spec("KP-011")]
    #[test]
    fn a_large_span_defers_rather_than_overrunning_the_budget() {
        // Measured: a 4KB span costs ~89µs to hash and probe in full, well over
        // the 50µs per-call invariant. The tier must not spend that on the
        // caller's thread — it stops and hands the span to asynchronous
        // scoring, which is the degradation KP-011 specifies.
        let tier = tier();
        let large = std::iter::repeat_n(CLASSIFIED, 60)
            .collect::<Vec<_>>()
            .join(" ");

        let started = Instant::now();
        let report = tier.examine(&large);
        let elapsed = started.elapsed();

        assert!(
            report.is_unavailable(),
            "the tier spent the whole span inline"
        );
        assert!(
            elapsed < TIER_BUDGET * 4,
            "deferral took {elapsed:?}, far past the {TIER_BUDGET:?} budget"
        );
    }

    #[spec("KP-011")]
    #[test]
    fn evidence_records_the_tier_even_when_it_found_nothing() {
        let evidence = evidence_for(&tier(), "1.0.0", "unrelated prose about the weather");

        assert_eq!(evidence.tiers.len(), 1);
        assert!(!evidence.has_gap());
    }
}
