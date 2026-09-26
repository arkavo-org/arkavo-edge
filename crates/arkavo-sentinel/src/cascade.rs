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

    /// Whether this tier can answer at all in this configuration.
    ///
    /// SENT-013's edge case: a tier that is permanently absent on this node is
    /// a configuration state, not an error per call. The cascade drops such a
    /// tier and records the absence once, because leaving it in would make it
    /// report a gap on every span — and a gap is a reason to hold, so an
    /// optional tier nobody provisioned would hold everything forever.
    fn is_available(&self) -> bool {
        true
    }

    /// Whether this tier judges spans too short for other tiers. A completed
    /// report from such a tier closes their out-of-scope gaps.
    fn covers_short_spans(&self) -> bool {
        false
    }

    /// Examine a span, stopping at the cascade's deadline.
    fn examine_until(&self, text: &str, deadline: Instant) -> TierReport;

    /// Examine a span with no deadline, for the path a deferral hands it to.
    fn examine_unbudgeted(&self, text: &str) -> TierReport;
}

/// Ordered tiers over one taxonomy version.
pub struct Cascade {
    tiers: Vec<Arc<dyn CascadeTier>>,
    /// Tiers not provisioned on this node, kept so an operator can see what
    /// this cascade is not covering without it costing a gap per span.
    absent: Vec<String>,
    taxonomy_version: String,
    budget: Duration,
}

impl Cascade {
    pub fn new(taxonomy_version: impl Into<String>) -> Self {
        Self {
            tiers: Vec::new(),
            absent: Vec::new(),
            taxonomy_version: taxonomy_version.into(),
            budget: CASCADE_BUDGET,
        }
    }

    /// Append a tier. Order is the cascade's contract, so tiers run in the
    /// order they were added rather than in an order chosen at run time.
    ///
    /// A tier that reports itself unavailable is recorded as absent rather than
    /// added, so its absence costs one operator log line instead of a gap on
    /// every span (SENT-013).
    #[must_use]
    pub fn with_tier(mut self, tier: Arc<dyn CascadeTier>) -> Self {
        if !tier.is_available() {
            tracing::info!(
                tier = tier.name(),
                "tier is not provisioned on this node; the cascade runs without it"
            );
            self.absent.push(tier.name().to_string());
            return self;
        }
        self.tiers.push(tier);
        self
    }

    /// Tiers this cascade is running without.
    pub fn absent_tiers(&self) -> &[String] {
        &self.absent
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
            let mut report = tier.examine_until(text, deadline);
            report.covers_short_spans = tier.covers_short_spans();
            evidence.push_tier(report);
        }
        evidence
    }

    /// Run the cascade off the hot path, where a deferral is resolved.
    pub fn inspect_unbudgeted(&self, text: &str) -> ClassificationEvidence {
        let mut evidence = ClassificationEvidence::new(&self.taxonomy_version);
        for tier in &self.tiers {
            let mut report = tier.examine_unbudgeted(text);
            report.covers_short_spans = tier.covers_short_spans();
            evidence.push_tier(report);
        }
        evidence
    }
}

impl CascadeTier for arkavo_fingerprint::ReferenceTier {
    fn name(&self) -> &str {
        arkavo_fingerprint::TIER_NAME
    }

    fn is_available(&self) -> bool {
        self.is_loaded()
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

    fn is_available(&self) -> bool {
        self.is_loaded()
    }

    fn examine_until(&self, text: &str, deadline: Instant) -> TierReport {
        arkavo_fingerprint::NearDuplicateTier::examine_until(self, text, deadline)
    }

    fn examine_unbudgeted(&self, text: &str) -> TierReport {
        arkavo_fingerprint::NearDuplicateTier::examine_unbudgeted(self, text)
    }
}

impl CascadeTier for arkavo_fingerprint::SemanticTier {
    fn name(&self) -> &str {
        arkavo_fingerprint::SEMANTIC_TIER_NAME
    }

    fn covers_short_spans(&self) -> bool {
        true
    }

    fn examine_until(&self, text: &str, deadline: Instant) -> TierReport {
        arkavo_fingerprint::SemanticTier::examine_until(self, text, deadline)
    }

    fn examine_unbudgeted(&self, text: &str) -> TierReport {
        arkavo_fingerprint::SemanticTier::examine_unbudgeted(self, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::classification_evidence::{Confidence, LabelFinding};
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
    use arkavo_test_macros::spec;
    use std::fmt::Write as _;
    use std::sync::Mutex;

    /// A tier that records when it was consulted, so ordering is observable.
    struct Recording {
        name: String,
        order: Arc<Mutex<Vec<String>>>,
        finding: Option<LabelFinding>,
        available: bool,
        covers: bool,
    }

    impl Recording {
        fn new(name: &str, order: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                name: name.into(),
                order,
                finding: None,
                available: true,
                covers: false,
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

        fn covering(mut self) -> Self {
            self.covers = true;
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

        fn covers_short_spans(&self) -> bool {
            self.covers
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

        // An already-expired deadline: every provisioned tier must report a gap
        // rather than spend the caller's time.
        let evidence = cascade.inspect_until(&"word ".repeat(400), Instant::now());

        assert!(evidence.has_gap());
        assert_eq!(evidence.tiers.len(), 1, "a deferral is still recorded");
        assert_eq!(
            cascade.absent_tiers(),
            [arkavo_fingerprint::NEAR_TIER_NAME],
            "an unprovisioned tier is recorded as absent, not as a gap per span"
        );
    }

    /// SENT-013 edge case: absence is a configuration state, not an error per
    /// call. An unprovisioned tier must not make every span look incomplete.
    #[spec("SENT-013")]
    #[test]
    fn an_unprovisioned_tier_does_not_put_a_gap_in_every_span() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let cascade = Cascade::new("1.0.0")
            .with_tier(Arc::new(Recording::new("exact", order)))
            .with_tier(Arc::new(arkavo_fingerprint::NearDuplicateTier::unloaded(
                "no near index on this node",
            )));

        let evidence = cascade.inspect("some text");

        assert!(!evidence.has_gap());
        assert_eq!(cascade.absent_tiers().len(), 1);
    }

    #[test]
    fn the_cascade_stamps_short_span_coverage_on_each_report() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let cascade = Cascade::new("1.0.0")
            .with_tier(Arc::new(Recording::new("exact", order.clone())))
            .with_tier(Arc::new(Recording::new("semantic", order).covering()));

        let evidence = cascade.inspect_unbudgeted("ok");

        assert!(!evidence.tiers[0].covers_short_spans);
        assert!(evidence.tiers[1].covers_short_spans);
    }

    /// Deterministic stand-in for a real embedder. Arkavo-sentinel cannot see
    /// arkavo-fingerprint's own `embed::tests::BagOfWords` (it is
    /// `pub(crate)` there), so this is a second copy, local to this test —
    /// hashing with `std`'s `DefaultHasher` rather than blake3 so this crate
    /// does not need to add a dependency just to exercise the cascade wiring.
    struct BagOfWords;

    impl arkavo_fingerprint::Embedder for BagOfWords {
        fn digest(&self) -> &'static str {
            "test-digest"
        }

        fn pooling(&self) -> arkavo_fingerprint::EmbeddingPooling {
            arkavo_fingerprint::EmbeddingPooling::Last
        }

        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
            use std::hash::{Hash, Hasher};
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; 64];
                    for w in t.split_whitespace() {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        w.to_lowercase().hash(&mut hasher);
                        v[(hasher.finish() as usize) % 64] += 1.0;
                    }
                    v
                })
                .collect())
        }
    }

    /// SENT-006/SENT-002: a span the near-duplicate tier calls out of scope
    /// must not sit in the evidence as a gap when the semantic tier — which
    /// covers short spans — completed on the same span.
    #[test]
    fn a_short_span_out_of_scope_for_near_duplicate_is_covered_by_semantic() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let near_key = Arc::new(
            arkavo_fingerprint::IndexKey::derive(&[11u8; 32], "cascade-semantic-tests")
                .expect("derive"),
        );
        let long_document = (0..140).fold(String::new(), |mut text, i| {
            let _ = write!(text, "t{i} ");
            text
        });
        let mut near_builder = arkavo_fingerprint::NearDuplicateIndex::builder(&near_key, "1.0.0");
        near_builder.add_document(
            &near_key,
            &long_document,
            arkavo_fingerprint::EntryMeta {
                category: DataCategory::Internal,
                sensitivity: SensitivityLevel::Confidential,
                source_family: "board".into(),
            },
        );
        let near_tier =
            arkavo_fingerprint::NearDuplicateTier::loaded(Arc::new(near_builder.build()), near_key);

        let mut semantic_builder = arkavo_fingerprint::SemanticIndexBuilder::new(
            "1.0.0",
            arkavo_fingerprint::EmbedderRecord {
                source: "o/m/f.gguf".into(),
                sha256: "test-digest".into(),
                pooling: arkavo_fingerprint::EmbeddingPooling::Last,
            },
        );
        semantic_builder
            .add_document(
                &BagOfWords,
                "the northwind acquisition closes in march with a hidden indemnity clause",
                DataCategory::Internal,
                SensitivityLevel::Confidential,
                "board",
            )
            .unwrap();
        semantic_builder
            .add_anchor(&BagOfWords, "oxycodone prescribing information")
            .unwrap();
        let semantic_index = Arc::new(semantic_builder.build().unwrap());
        let calibration = arkavo_fingerprint::SemanticCalibration {
            detector_version: arkavo_fingerprint::SemanticCalibration::detector_version_for(
                "test-digest",
            ),
            taxonomy_version: "1.0.0".into(),
            margins: std::collections::BTreeMap::from([(
                arkavo_fingerprint::label_key(
                    DataCategory::Internal,
                    SensitivityLevel::Confidential,
                ),
                0.3,
            )]),
            embedder: semantic_index.embedder.clone(),
        };
        let semantic_tier = arkavo_fingerprint::SemanticTier::new(
            semantic_index,
            calibration,
            Arc::new(BagOfWords),
        )
        .unwrap();

        let cascade = Cascade::new("1.0.0")
            .with_tier(Arc::new(Recording::new("exact", order)) as Arc<dyn CascadeTier>)
            .with_tier(Arc::new(near_tier) as Arc<dyn CascadeTier>)
            .with_tier(Arc::new(semantic_tier) as Arc<dyn CascadeTier>);

        let evidence = cascade.inspect_unbudgeted("ok");

        assert!(!evidence.has_gap());
    }
}
