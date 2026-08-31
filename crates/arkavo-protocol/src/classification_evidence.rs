//! What a classification tier reports (SENT-002, KP-009).
//!
//! [`ClassifiedDatum`] cannot carry this. Its `DatumType` is a closed enum that
//! answers `category()` and `default_sensitivity()` from the variant itself, so
//! a finding whose category comes from an index entry rather than from a regex
//! has nowhere to put it — nor anywhere to put the confidence, the tier
//! version, or which source family matched.
//!
//! Evidence is not a verdict. A tier says what it saw and how sure it is; the
//! policy decision point decides what that means. Keeping the two apart is the
//! whole point of the sentinel design, and it starts here.
//!
//! [`ClassifiedDatum`]: crate::data_classification::ClassifiedDatum

use serde::{Deserialize, Serialize};

use crate::data_classification::{DataCategory, SensitivityLevel};

/// A calibrated probability in `0.0..=1.0`.
///
/// Calibrated, not raw: a raw model score is not comparable across labels or
/// across detector versions, and a threshold applied to one is meaningless.
/// Construction clamps rather than rejecting, because a tier that returns a
/// nonsense score should be visible in the evidence rather than fatal to the
/// call it was scoring.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Confidence(f32);

impl Confidence {
    pub const CERTAIN: Confidence = Confidence(1.0);

    pub fn new(value: f32) -> Self {
        if value.is_nan() {
            return Confidence(0.0);
        }
        Confidence(value.clamp(0.0, 1.0))
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

/// One label a tier inferred, with everything needed to defend it later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelFinding {
    pub category: DataCategory,
    pub sensitivity: SensitivityLevel,
    pub confidence: Confidence,
    /// What fired, in terms an auditor can follow: a pattern name, a matched
    /// shingle count, a model label.
    pub signal: String,
    /// Which corpus family the match came from, when a reference tier
    /// contributed. Absent for tiers that match on content alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_family: Option<String>,
    /// Byte span within the inspected text, when the tier can localize it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<(usize, usize)>,
}

impl LabelFinding {
    pub fn new(
        category: DataCategory,
        sensitivity: SensitivityLevel,
        confidence: Confidence,
        signal: impl Into<String>,
    ) -> Self {
        Self {
            category,
            sensitivity,
            confidence,
            signal: signal.into(),
            source_family: None,
            span: None,
        }
    }

    #[must_use]
    pub fn from_family(mut self, family: impl Into<String>) -> Self {
        self.source_family = Some(family.into());
        self
    }

    #[must_use]
    pub fn at(mut self, start: usize, end: usize) -> Self {
        self.span = Some((start, end));
        self
    }
}

/// What one tier had to say.
///
/// `NoMatch` and `Unavailable` are distinct on purpose (SENT-002 edge case): a
/// tier that looked and found nothing is evidence of absence, a tier that could
/// not look is evidence of nothing at all, and collapsing them lets an outage
/// read as a clean bill of health.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum TierOutcome {
    Matched { findings: Vec<LabelFinding> },
    NoMatch,
    Unavailable { reason: String },
}

/// One tier's contribution, recorded whether or not it matched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierReport {
    pub tier: String,
    /// Version of the detector that produced this, so a past decision stays
    /// reconstructable after the detector changes.
    pub version: String,
    #[serde(flatten)]
    pub outcome: TierOutcome,
}

impl TierReport {
    pub fn matched(
        tier: impl Into<String>,
        version: impl Into<String>,
        findings: Vec<LabelFinding>,
    ) -> Self {
        let outcome = if findings.is_empty() {
            TierOutcome::NoMatch
        } else {
            TierOutcome::Matched { findings }
        };
        Self {
            tier: tier.into(),
            version: version.into(),
            outcome,
        }
    }

    pub fn unavailable(
        tier: impl Into<String>,
        version: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tier: tier.into(),
            version: version.into(),
            outcome: TierOutcome::Unavailable {
                reason: reason.into(),
            },
        }
    }

    pub fn findings(&self) -> &[LabelFinding] {
        match &self.outcome {
            TierOutcome::Matched { findings } => findings,
            TierOutcome::NoMatch | TierOutcome::Unavailable { .. } => &[],
        }
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self.outcome, TierOutcome::Unavailable { .. })
    }
}

/// Everything the cascade observed about one span.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClassificationEvidence {
    /// Taxonomy the labels are expressed in. Without it a stored finding cannot
    /// be read years later, because `Confidential` may not mean what it did.
    pub taxonomy_version: String,
    /// Every tier consulted, in cascade order, matched or not.
    pub tiers: Vec<TierReport>,
}

impl ClassificationEvidence {
    pub fn new(taxonomy_version: impl Into<String>) -> Self {
        Self {
            taxonomy_version: taxonomy_version.into(),
            tiers: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_tier(mut self, report: TierReport) -> Self {
        self.tiers.push(report);
        self
    }

    pub fn push_tier(&mut self, report: TierReport) {
        self.tiers.push(report);
    }

    pub fn findings(&self) -> impl Iterator<Item = &LabelFinding> {
        self.tiers.iter().flat_map(TierReport::findings)
    }

    /// Whether any tier could not be consulted. A gap is a reason to treat the
    /// result as incomplete, not as clean.
    pub fn has_gap(&self) -> bool {
        self.tiers.iter().any(TierReport::is_unavailable)
    }

    /// Highest sensitivity any tier reported at or above `threshold`.
    ///
    /// `None` when nothing cleared the threshold — which is not the same as
    /// `Public`, and callers must not read it that way.
    pub fn sensitivity_at(&self, threshold: Confidence) -> Option<SensitivityLevel> {
        self.findings()
            .filter(|f| f.confidence >= threshold)
            .map(|f| f.sensitivity)
            .max()
    }

    /// Categories reported at or above `threshold`.
    pub fn categories_at(&self, threshold: Confidence) -> Vec<DataCategory> {
        let mut categories: Vec<DataCategory> = self
            .findings()
            .filter(|f| f.confidence >= threshold)
            .map(|f| f.category)
            .collect();
        categories.sort_unstable();
        categories.dedup();
        categories
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(category: DataCategory, level: SensitivityLevel, c: f32) -> LabelFinding {
        LabelFinding::new(category, level, Confidence::new(c), "test")
    }

    #[test]
    fn confidence_is_clamped_rather_than_rejected() {
        assert_eq!(Confidence::new(1.7).value(), 1.0);
        assert_eq!(Confidence::new(-0.2).value(), 0.0);
        assert_eq!(Confidence::new(f32::NAN).value(), 0.0);
    }

    #[test]
    fn a_tier_that_looked_and_found_nothing_is_recorded() {
        let evidence = ClassificationEvidence::new("1.0.0").with_tier(TierReport::matched(
            "exact",
            "1",
            Vec::new(),
        ));

        assert_eq!(evidence.tiers.len(), 1);
        assert_eq!(evidence.tiers[0].outcome, TierOutcome::NoMatch);
        assert!(!evidence.has_gap());
    }

    #[test]
    fn a_tier_that_could_not_look_is_a_gap_not_a_clean_result() {
        // Collapsing these would let an index outage read as "nothing found".
        let evidence = ClassificationEvidence::new("1.0.0").with_tier(TierReport::unavailable(
            "exact",
            "1",
            "index not loaded",
        ));

        assert!(evidence.has_gap());
        assert_eq!(evidence.sensitivity_at(Confidence::new(0.0)), None);
    }

    #[test]
    fn nothing_above_threshold_is_not_public() {
        let evidence = ClassificationEvidence::new("1.0.0").with_tier(TierReport::matched(
            "sentinel",
            "1",
            vec![finding(DataCategory::Pii, SensitivityLevel::Internal, 0.3)],
        ));

        assert_eq!(evidence.sensitivity_at(Confidence::new(0.9)), None);
        assert_eq!(
            evidence.sensitivity_at(Confidence::new(0.2)),
            Some(SensitivityLevel::Internal)
        );
    }

    #[test]
    fn the_highest_sensitivity_above_threshold_wins() {
        let evidence = ClassificationEvidence::new("1.0.0").with_tier(TierReport::matched(
            "exact",
            "1",
            vec![
                finding(DataCategory::Internal, SensitivityLevel::Internal, 0.99),
                finding(DataCategory::Healthcare, SensitivityLevel::Restricted, 0.95),
            ],
        ));

        assert_eq!(
            evidence.sensitivity_at(Confidence::new(0.9)),
            Some(SensitivityLevel::Restricted)
        );
        let mut expected = vec![DataCategory::Healthcare, DataCategory::Internal];
        expected.sort_unstable();
        assert_eq!(evidence.categories_at(Confidence::new(0.9)), expected);
    }

    #[test]
    fn evidence_round_trips_with_the_tier_that_found_nothing_intact() {
        let evidence = ClassificationEvidence::new("1.0.0")
            .with_tier(TierReport::matched("exact", "1", Vec::new()))
            .with_tier(TierReport::matched(
                "sentinel",
                "2",
                vec![
                    finding(DataCategory::Financial, SensitivityLevel::Confidential, 0.8)
                        .from_family("invoices"),
                ],
            ));

        let json = serde_json::to_string(&evidence).expect("serialize");
        let back: ClassificationEvidence = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back, evidence);
        assert_eq!(back.tiers.len(), 2);
        assert_eq!(
            back.findings().next().unwrap().source_family.as_deref(),
            Some("invoices")
        );
    }
}
