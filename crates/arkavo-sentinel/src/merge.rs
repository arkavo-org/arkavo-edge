//! Merging inferred labels into known ones (SENT-003).
//!
//! The rule is one sentence long and the whole design rests on it: inference
//! may add restrictions and may never remove them. Sensitivity becomes the
//! maximum of known and inferred, categories become the union, and a known
//! label the sentinel disagrees with stands unchanged.
//!
//! The merge is over the **whole payload**, not per matched datum. A payload
//! holding a credential and an email address is a credential payload; deciding
//! about the email on its own is how the low-sensitivity half of a buffer ends
//! up answering for the other half.

use arkavo_protocol::classification_evidence::{ClassificationEvidence, Confidence};
use arkavo_protocol::data_classification::SensitivityLevel;
use arkavo_protocol::taint::{TaintLabel, TaintSet};

/// Source id recorded for labels that came from inference rather than from a
/// declared origin, so an auditor can tell the two apart in a taint set.
pub const INFERRED_SOURCE: &str = "sentinel:inferred";

/// Merge evidence into a taint set, keeping only findings at or above
/// `threshold`.
///
/// Returns whether the set changed, which is what tells a caller that a payload
/// it already labelled has been raised.
pub fn merge_evidence(
    taint: &mut TaintSet,
    evidence: &ClassificationEvidence,
    threshold: Confidence,
) -> bool {
    let categories = evidence.categories_at(threshold);
    let sensitivity = evidence.sensitivity_at(threshold);
    if categories.is_empty() && sensitivity.is_none() {
        return false;
    }
    let before = taint.clone();
    // A floor of Public when nothing cleared the threshold, because `insert`
    // unions with whatever the set already holds for this source: a lower
    // inference cannot pull an existing label down.
    let label = TaintLabel::new(
        INFERRED_SOURCE,
        categories,
        sensitivity.unwrap_or(SensitivityLevel::Public),
    );
    taint.insert(label);
    *taint != before
}

/// Evidence that could not be completed, which a policy layer must treat as a
/// reason to hold rather than as a clean result (SENT-013).
pub fn has_gap(evidence: &ClassificationEvidence) -> bool {
    evidence.has_gap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::classification_evidence::{LabelFinding, TierReport};
    use arkavo_protocol::data_classification::DataCategory;
    use arkavo_test_macros::spec;

    fn evidence_of(findings: Vec<LabelFinding>) -> ClassificationEvidence {
        ClassificationEvidence::new("1.0.0").with_tier(TierReport::matched("t", "1", findings))
    }

    fn finding(
        category: DataCategory,
        sensitivity: SensitivityLevel,
        confidence: f32,
    ) -> LabelFinding {
        LabelFinding::new(category, sensitivity, Confidence::new(confidence), "signal")
    }

    /// SENT-003: sensitivity is the maximum, categories are the union.
    #[spec("SENT-003")]
    #[test]
    fn inference_raises_sensitivity_and_unions_categories() {
        let mut taint = TaintSet::from_label(TaintLabel::new(
            "tool:read",
            [DataCategory::Internal],
            SensitivityLevel::Internal,
        ));

        let changed = merge_evidence(
            &mut taint,
            &evidence_of(vec![finding(
                DataCategory::Credentials,
                SensitivityLevel::Restricted,
                0.9,
            )]),
            Confidence::new(0.5),
        );

        assert!(changed);
        assert_eq!(taint.sensitivity(), SensitivityLevel::Restricted);
        assert!(taint.contains_category(DataCategory::Credentials));
        assert!(taint.contains_category(DataCategory::Internal));
    }

    /// SENT-003 edge case: a lower inference leaves the known label standing.
    #[spec("SENT-003")]
    #[test]
    fn a_lower_inference_never_downgrades_a_known_label() {
        let mut taint = TaintSet::from_label(TaintLabel::new(
            "tool:read",
            [DataCategory::Credentials],
            SensitivityLevel::Restricted,
        ));

        merge_evidence(
            &mut taint,
            &evidence_of(vec![finding(
                DataCategory::Public,
                SensitivityLevel::Public,
                0.99,
            )]),
            Confidence::new(0.5),
        );

        assert_eq!(taint.sensitivity(), SensitivityLevel::Restricted);
    }

    /// SENT-003 edge case: a payload mixing a credential and an email address
    /// is evaluated at the credential level, because the merge is over the
    /// payload rather than over one matched datum.
    #[spec("SENT-003")]
    #[test]
    fn a_mixed_payload_carries_the_highest_label_in_it() {
        let mut taint = TaintSet::new();

        merge_evidence(
            &mut taint,
            &evidence_of(vec![
                finding(DataCategory::Pii, SensitivityLevel::Internal, 0.9),
                finding(DataCategory::Credentials, SensitivityLevel::Restricted, 0.9),
            ]),
            Confidence::new(0.5),
        );

        assert_eq!(taint.sensitivity(), SensitivityLevel::Restricted);
        assert!(taint.contains_category(DataCategory::Pii));
    }

    /// Findings below the threshold are not evidence yet. Merging them would
    /// make the threshold decorative.
    #[spec("SENT-004")]
    #[test]
    fn a_finding_below_the_threshold_does_not_merge() {
        let mut taint = TaintSet::new();

        let changed = merge_evidence(
            &mut taint,
            &evidence_of(vec![finding(
                DataCategory::Credentials,
                SensitivityLevel::Restricted,
                0.2,
            )]),
            Confidence::new(0.8),
        );

        assert!(!changed);
        assert!(taint.is_empty());
    }

    /// SENT-013: a tier that could not answer leaves a gap in the evidence, and
    /// an empty result from a cascade with a gap is not a clean result.
    #[spec("SENT-013")]
    #[test]
    fn an_unavailable_tier_leaves_a_visible_gap() {
        let evidence = ClassificationEvidence::new("1.0.0").with_tier(TierReport::unavailable(
            "t",
            "1",
            "not loaded",
        ));

        assert!(has_gap(&evidence));
    }
}
