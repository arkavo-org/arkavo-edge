//! The semantic tier as the cascade sees it.
//!
//! Every other tier in this crate answers inline, inside a shared per-call
//! deadline. This one never does: an embedding call is a model invocation, not
//! a hash-map probe, and a tier that could be persuaded to run inline for a
//! short enough span is a tier that can be made to run inline for every span —
//! the same rationale `SentinelTier` uses for its own synchronous path. So
//! `examine_until` always defers, and the only place semantic scoring actually
//! runs is `examine_unbudgeted`, off the caller's thread.
//!
//! What comes back on a match is deliberately thin: a label, a family and a
//! fixed confidence, never the raw margin. The margin is the whole reason this
//! tier's own threshold exists — publishing it would let a caller learn how
//! close a near-miss came, which is a probe an attacker can iterate on.

use std::sync::Arc;
use std::time::Instant;

use arkavo_protocol::classification_evidence::{Confidence, LabelFinding, TierReport};

use crate::embed::{Embedder, embed_units};
use crate::semantic_calibration::SemanticCalibration;
use crate::semantic_index::SemanticIndex;

pub const SEMANTIC_TIER_NAME: &str = "semantic";

/// Fixed rather than derived from the margin.
///
/// The calibrated threshold is already the decision boundary, so every match
/// above it carries the tier's own confidence in that boundary rather than a
/// distance from it.
pub const SEMANTIC_CONFIDENCE: f32 = 0.9;

pub struct SemanticTier {
    pub(crate) index: Arc<SemanticIndex>,
    pub(crate) calibration: SemanticCalibration,
    embedder: Arc<dyn Embedder>,
}

impl SemanticTier {
    /// Refuses construction rather than scoring with a mismatched embedder or
    /// a calibration built for a different index: a margin from one embedding
    /// space compared against a threshold from another is not wrong, it is
    /// meaningless, and there is no report that could make that safe to ship.
    pub fn new(
        index: Arc<SemanticIndex>,
        calibration: SemanticCalibration,
        embedder: Arc<dyn Embedder>,
    ) -> Result<Self, String> {
        if embedder.digest() != index.embedder.sha256 {
            return Err(format!(
                "embedder digest {} does not match the index's embedder {}",
                embedder.digest(),
                index.embedder.sha256
            ));
        }
        if embedder.pooling() != index.embedder.pooling {
            return Err(
                "embedder pooling does not match the pooling the index was built with".to_string(),
            );
        }
        let expected_version = SemanticCalibration::detector_version_for(&index.embedder.sha256);
        if calibration.detector_version != expected_version {
            return Err(format!(
                "calibration detector version {} does not match {expected_version}",
                calibration.detector_version
            ));
        }
        if calibration.taxonomy_version != index.taxonomy_version {
            return Err(format!(
                "calibration taxonomy version {} does not match the index's {}",
                calibration.taxonomy_version, index.taxonomy_version
            ));
        }
        // Scoring skips a label with no threshold, so an index label the
        // calibration left out would never fire — a hole opened silently at
        // load rather than a gap anyone sees.
        if let Some(missing) = index
            .labels()
            .into_iter()
            .find(|label| !calibration.margins.contains_key(label))
        {
            return Err(format!(
                "the index holds label {missing} but the calibration has no threshold for it"
            ));
        }
        Ok(Self {
            index,
            calibration,
            embedder,
        })
    }

    pub fn version(&self) -> String {
        let sha = &self.index.embedder.sha256;
        let prefix = &sha[..12.min(sha.len())];
        format!(
            "{}+{}+{prefix}",
            self.index.format_version, self.index.taxonomy_version
        )
    }

    /// Always defers: see the module doc for why running inline is never safe
    /// here, no matter how far off the deadline is.
    pub fn examine_until(&self, _text: &str, _deadline: Instant) -> TierReport {
        TierReport::unavailable(
            SEMANTIC_TIER_NAME,
            self.version(),
            "semantic scoring runs off the synchronous path; deferred",
        )
    }

    /// Score a span with no deadline, for the path a deferral hands it to.
    pub fn examine_unbudgeted(&self, text: &str) -> TierReport {
        match embed_units(&*self.embedder, text) {
            Err(e) => TierReport::unavailable(
                SEMANTIC_TIER_NAME,
                self.version(),
                format!("embedder failed: {e}"),
            ),
            Ok(units) => {
                let findings = match self.index.judge(&units, &self.calibration.margins) {
                    Some(m) => vec![
                        LabelFinding::new(
                            m.category,
                            m.sensitivity,
                            Confidence::new(SEMANTIC_CONFIDENCE),
                            "semantic match above the calibrated margin",
                        )
                        .from_family(m.family),
                    ],
                    None => Vec::new(),
                };
                TierReport::matched(SEMANTIC_TIER_NAME, self.version(), findings)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::tests::BagOfWords;
    use crate::semantic_index::{EmbedderRecord, SemanticIndexBuilder, label_key};
    use arkavo_protocol::classification_evidence::TierOutcome;
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
    use std::collections::BTreeMap;

    const SECRET: &str = "the northwind acquisition closes in march with a hidden indemnity clause";

    fn tier_with(threshold: f32) -> SemanticTier {
        let mut b = SemanticIndexBuilder::new(
            "1.0.0",
            EmbedderRecord {
                source: "o/m/f.gguf".into(),
                sha256: "test-digest".into(),
                pooling: crate::embed::EmbeddingPooling::Last,
            },
        );
        b.add_document(
            &BagOfWords,
            SECRET,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board",
        )
        .unwrap();
        b.add_anchor(&BagOfWords, "oxycodone prescribing information")
            .unwrap();
        let index = Arc::new(b.build().unwrap());
        let calibration = SemanticCalibration {
            detector_version: SemanticCalibration::detector_version_for("test-digest"),
            taxonomy_version: "1.0.0".into(),
            margins: BTreeMap::from([(
                label_key(DataCategory::Internal, SensitivityLevel::Confidential),
                threshold,
            )]),
            embedder: index.embedder.clone(),
        };
        SemanticTier::new(index, calibration, Arc::new(BagOfWords)).unwrap()
    }

    #[test]
    fn the_synchronous_path_always_defers() {
        let report =
            tier_with(0.3).examine_until("ok", Instant::now() + std::time::Duration::from_secs(60));
        assert!(report.is_unavailable());
    }

    #[test]
    fn a_match_is_a_finding_without_a_score() {
        let report = tier_with(0.3).examine_unbudgeted(SECRET);
        let finding = &report.findings()[0];
        assert_eq!(finding.sensitivity, SensitivityLevel::Confidential);
        assert_eq!(finding.source_family.as_deref(), Some("board"));
        assert!(
            !finding.signal.chars().any(|c| c.is_ascii_digit()),
            "no raw score in the signal"
        );
    }

    #[test]
    fn a_short_clean_span_is_judged_not_deferred() {
        let report = tier_with(0.3).examine_unbudgeted("ok");
        assert_eq!(report.outcome, TierOutcome::NoMatch);
    }

    #[test]
    fn empty_text_is_a_clean_result() {
        assert_eq!(
            tier_with(0.3).examine_unbudgeted("").outcome,
            TierOutcome::NoMatch
        );
    }

    #[test]
    fn an_embedder_failure_is_a_gap() {
        struct Broken;
        impl Embedder for Broken {
            fn digest(&self) -> &'static str {
                "test-digest"
            }
            fn pooling(&self) -> crate::embed::EmbeddingPooling {
                crate::embed::EmbeddingPooling::Last
            }
            fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, String> {
                Err("oom".into())
            }
        }
        let good = tier_with(0.3);
        let tier =
            SemanticTier::new(good.index.clone(), good.calibration, Arc::new(Broken)).unwrap();
        assert!(tier.examine_unbudgeted(SECRET).is_unavailable());
    }

    #[test]
    fn a_mismatched_embedder_is_refused_at_construction() {
        struct Other;
        impl Embedder for Other {
            fn digest(&self) -> &'static str {
                "other-digest"
            }
            fn pooling(&self) -> crate::embed::EmbeddingPooling {
                crate::embed::EmbeddingPooling::Last
            }
            fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, String> {
                Ok(Vec::new())
            }
        }
        let good = tier_with(0.3);
        assert!(SemanticTier::new(good.index.clone(), good.calibration, Arc::new(Other)).is_err());
    }

    #[test]
    fn a_label_without_a_threshold_is_refused_at_construction() {
        let mut b = SemanticIndexBuilder::new(
            "1.0.0",
            EmbedderRecord {
                source: "o/m/f.gguf".into(),
                sha256: "test-digest".into(),
                pooling: crate::embed::EmbeddingPooling::Last,
            },
        );
        b.add_document(
            &BagOfWords,
            SECRET,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board",
        )
        .unwrap();
        b.add_document(
            &BagOfWords,
            "quarterly payroll for the plant staff",
            DataCategory::Financial,
            SensitivityLevel::Restricted,
            "payroll",
        )
        .unwrap();
        b.add_anchor(&BagOfWords, "oxycodone prescribing information")
            .unwrap();
        let index = Arc::new(b.build().unwrap());
        let calibration = SemanticCalibration {
            detector_version: SemanticCalibration::detector_version_for("test-digest"),
            taxonomy_version: "1.0.0".into(),
            margins: BTreeMap::from([(
                label_key(DataCategory::Internal, SensitivityLevel::Confidential),
                0.3,
            )]),
            embedder: index.embedder.clone(),
        };
        let missing = label_key(DataCategory::Financial, SensitivityLevel::Restricted);
        let err = SemanticTier::new(index, calibration, Arc::new(BagOfWords))
            .err()
            .expect("an uncalibrated label must be refused");
        assert!(err.contains(&missing), "{err}");
    }

    #[test]
    fn thresholds_for_another_embedder_are_refused() {
        let good = tier_with(0.3);
        let mut cal = good.calibration.clone();
        cal.detector_version = SemanticCalibration::detector_version_for("someone-else");
        assert!(SemanticTier::new(good.index, cal, Arc::new(BagOfWords)).is_err());
    }
}
