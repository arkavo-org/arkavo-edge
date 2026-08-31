//! The classification cascade (SENT-006, SENT-013, SENT-016).
//!
//! Tiers run in order, cheapest first, against **one** deadline. That is the
//! part worth stating plainly: a per-tier budget means the cascade's cost is
//! the sum of the budgets, so adding a tier silently raises the per-call
//! overhead the SEQ invariant caps at 50µs. One deadline threaded through every
//! tier means adding a tier can only take time from the tiers after it.
//!
//! A tier that runs out of deadline reports a gap and the span is deferred to
//! the asynchronous path. That is the whole reason a gap is a distinct outcome
//! from a clean miss: a deferral is a promise that something still has to look,
//! and a cascade that lost that distinction would release uninspected content
//! every time it got busy.

use std::sync::Arc;
use std::time::{Duration, Instant};

use arkavo_protocol::classification_evidence::{ClassificationEvidence, TierReport};

/// Total synchronous budget for the whole cascade.
///
/// The SEQ invariant is 50µs per tool call for *everything* on that path, and
/// the pattern detector runs there too. Thirty is the cascade's share of it.
pub const CASCADE_BUDGET: Duration = Duration::from_micros(30);

/// One stage of the cascade.
///
/// Both methods return a report rather than a verdict. A tier that could decide
/// would be a second authorization engine, which is precisely what the sentinel
/// design exists to avoid.
pub trait CascadeTier: Send + Sync {
    fn name(&self) -> &str;

    /// Examine a span, stopping at the cascade's deadline.
    fn examine_until(&self, text: &str, deadline: Instant) -> TierReport;

    /// Examine a span with no deadline, for the path a deferral hands it to.
    fn examine_unbudgeted(&self, text: &str) -> TierReport;
}

/// Ordered tiers over one taxonomy version.
pub struct Cascade {
    tiers: Vec<Arc<dyn CascadeTier>>,
    taxonomy_version: String,
    budget: Duration,
}

impl Cascade {
    pub fn new(taxonomy_version: impl Into<String>) -> Self {
        Self {
            tiers: Vec::new(),
            taxonomy_version: taxonomy_version.into(),
            budget: CASCADE_BUDGET,
        }
    }

    /// Append a tier. Order is the cascade's contract, so tiers run in the
    /// order they were added rather than in an order chosen at run time.
    #[must_use]
    pub fn with_tier(mut self, tier: Arc<dyn CascadeTier>) -> Self {
        self.tiers.push(tier);
        self
    }

    #[must_use]
    pub fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    pub fn taxonomy_version(&self) -> &str {
        &self.taxonomy_version
    }

    pub fn tier_names(&self) -> Vec<&str> {
        self.tiers.iter().map(|t| t.name()).collect()
    }

    /// Run the cascade inside the per-call budget.
    ///
    /// Every tier is consulted and every tier is recorded, including one that
    /// found nothing (SENT-002) and one that ran out of deadline (SENT-013).
    /// Tiers after an exhausted deadline are not skipped silently — they report
    /// the gap themselves, because a cascade whose evidence shrinks under load
    /// looks cleaner exactly when it is doing less work.
    pub fn inspect(&self, text: &str) -> ClassificationEvidence {
        self.inspect_until(text, Instant::now() + self.budget)
    }

    pub fn inspect_until(&self, text: &str, deadline: Instant) -> ClassificationEvidence {
        let mut evidence = ClassificationEvidence::new(&self.taxonomy_version);
        for tier in &self.tiers {
            evidence.push_tier(tier.examine_until(text, deadline));
        }
        evidence
    }

    /// Run the cascade off the hot path, where a deferral is resolved.
    pub fn inspect_unbudgeted(&self, text: &str) -> ClassificationEvidence {
        let mut evidence = ClassificationEvidence::new(&self.taxonomy_version);
        for tier in &self.tiers {
            evidence.push_tier(tier.examine_unbudgeted(text));
        }
        evidence
    }
}

impl CascadeTier for arkavo_fingerprint::ReferenceTier {
    fn name(&self) -> &str {
        arkavo_fingerprint::TIER_NAME
    }

    fn examine_until(&self, text: &str, deadline: Instant) -> TierReport {
        arkavo_fingerprint::ReferenceTier::examine_until(self, text, deadline)
    }

    fn examine_unbudgeted(&self, text: &str) -> TierReport {
        arkavo_fingerprint::ReferenceTier::examine_unbudgeted(self, text)
    }
}

impl CascadeTier for arkavo_fingerprint::NearDuplicateTier {
    fn name(&self) -> &str {
        arkavo_fingerprint::NEAR_TIER_NAME
    }

    fn examine_until(&self, text: &str, deadline: Instant) -> TierReport {
        arkavo_fingerprint::NearDuplicateTier::examine_until(self, text, deadline)
    }

    fn examine_unbudgeted(&self, text: &str) -> TierReport {
        arkavo_fingerprint::NearDuplicateTier::examine_unbudgeted(self, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::classification_evidence::{Confidence, LabelFinding};
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
    use arkavo_test_macros::spec;
    use std::sync::Mutex;

    /// A tier that records when it was consulted, so ordering is observable.
    struct Recording {
        name: String,
        order: Arc<Mutex<Vec<String>>>,
        finding: Option<LabelFinding>,
        available: bool,
    }

    impl Recording {
        fn new(name: &str, order: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                name: name.into(),
                order,
                finding: None,
                available: true,
            }
        }

        fn finding(mut self, sensitivity: SensitivityLevel) -> Self {
            self.finding = Some(LabelFinding::new(
                DataCategory::Internal,
                sensitivity,
                Confidence::CERTAIN,
                "test",
            ));
            self
        }

        fn unavailable(mut self) -> Self {
            self.available = false;
            self
        }

        fn report(&self) -> TierReport {
            self.order.lock().expect("lock").push(self.name.clone());
            if !self.available {
                return TierReport::unavailable(&self.name, "1", "not loaded");
            }
            TierReport::matched(&self.name, "1", self.finding.clone().into_iter().collect())
        }
    }

    impl CascadeTier for Recording {
        fn name(&self) -> &str {
            &self.name
        }

        fn examine_until(&self, _text: &str, _deadline: Instant) -> TierReport {
            self.report()
        }

        fn examine_unbudgeted(&self, _text: &str) -> TierReport {
            self.report()
        }
    }

    /// SENT-006: the keyed exact tier runs first and the near-duplicate tier
    /// second, in the order the cascade was built.
    #[spec("SENT-006")]
    #[test]
    fn tiers_run_in_the_order_they_were_added() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let cascade = Cascade::new("1.0.0")
            .with_tier(Arc::new(Recording::new("exact", order.clone())))
            .with_tier(Arc::new(Recording::new("near", order.clone())))
            .with_tier(Arc::new(Recording::new("sentinel", order.clone())));

        cascade.inspect("some text");

        assert_eq!(*order.lock().expect("lock"), ["exact", "near", "sentinel"]);
    }

    /// SENT-006: evidence names which tier produced each label.
    #[spec("SENT-006")]
    #[test]
    fn evidence_names_the_tier_that_produced_each_label() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let cascade = Cascade::new("1.0.0")
            .with_tier(Arc::new(Recording::new("exact", order.clone())))
            .with_tier(
                Arc::new(Recording::new("near", order).finding(SensitivityLevel::Restricted))
                    as Arc<dyn CascadeTier>,
            );

        let evidence = cascade.inspect("some text");

        let producing: Vec<&str> = evidence
            .tiers
            .iter()
            .filter(|t| !t.findings().is_empty())
            .map(|t| t.tier.as_str())
            .collect();
        assert_eq!(producing, ["near"]);
    }

    /// SENT-006 edge case: an earlier tier firing the maximum label does not
    /// stop later tiers, and later tiers cannot lower the result.
    #[spec("SENT-006")]
    #[test]
    fn a_later_tier_cannot_lower_what_an_earlier_one_found() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let cascade = Cascade::new("1.0.0")
            .with_tier(Arc::new(
                Recording::new("exact", order.clone()).finding(SensitivityLevel::Restricted),
            ) as Arc<dyn CascadeTier>)
            .with_tier(Arc::new(
                Recording::new("near", order.clone()).finding(SensitivityLevel::Public),
            ) as Arc<dyn CascadeTier>);

        let evidence = cascade.inspect("some text");

        assert_eq!(
            order.lock().expect("lock").len(),
            2,
            "later tiers still run"
        );
        assert_eq!(
            evidence.sensitivity_at(Confidence::new(0.5)),
            Some(SensitivityLevel::Restricted)
        );
    }

    /// SENT-002 edge case: a tier that contributed no signal is recorded as
    /// consulted, not omitted.
    #[spec("SENT-002")]
    #[test]
    fn every_tier_is_recorded_including_the_ones_that_found_nothing() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let cascade = Cascade::new("1.0.0")
            .with_tier(Arc::new(Recording::new("exact", order.clone())))
            .with_tier(Arc::new(Recording::new("near", order)));

        let evidence = cascade.inspect("some text");

        assert_eq!(evidence.tiers.len(), 2);
    }

    /// SENT-013: a tier that could not run leaves a gap, which is not a clean
    /// result. The cascade must not paper over it.
    #[spec("SENT-013")]
    #[test]
    fn an_unavailable_tier_leaves_the_evidence_with_a_gap() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let cascade = Cascade::new("1.0.0")
            .with_tier(Arc::new(Recording::new("exact", order.clone())))
            .with_tier(
                Arc::new(Recording::new("near", order).unavailable()) as Arc<dyn CascadeTier>
            );

        let evidence = cascade.inspect("some text");

        assert!(evidence.has_gap());
    }

    /// SENT-016: the cascade's synchronous cost is bounded by one deadline for
    /// the whole chain, not by a budget per tier.
    #[spec("SENT-016")]
    #[test]
    fn the_whole_cascade_shares_one_deadline() {
        let key = Arc::new(
            arkavo_fingerprint::IndexKey::derive(&[3u8; 32], "cascade-tests").expect("derive"),
        );
        let mut builder = arkavo_fingerprint::ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            "the acquisition of northwind holdings closes in the third quarter",
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board",
        );
        let index = Arc::new(builder.build());
        let cascade = Cascade::new("1.0.0")
            .with_tier(
                Arc::new(arkavo_fingerprint::ReferenceTier::loaded(index, key))
                    as Arc<dyn CascadeTier>,
            )
            .with_tier(Arc::new(arkavo_fingerprint::NearDuplicateTier::unloaded(
                "none",
            )));

        // An already-expired deadline: every tier must report a gap rather than
        // spend the caller's time.
        let evidence = cascade.inspect_until(&"word ".repeat(400), Instant::now());

        assert!(evidence.has_gap());
        assert_eq!(evidence.tiers.len(), 2, "a deferral is still recorded");
    }
}
