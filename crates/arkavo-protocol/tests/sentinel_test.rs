#![cfg(feature = "taint")]
//! SENT-001, SENT-002, SENT-003, SENT-013: the split between labelling and
//! authorizing.
//!
//! `DlpPolicy` fuses detection and authorization: it looks at one datum and
//! answers Allow or Block. The sentinel splits those apart — a tier produces
//! evidence, the policy decision point authorizes. These were tripwires against
//! the fused design; they now assert the separated one, and the `DlpPolicy`
//! tests that remain document the fused surface still in place beside it.

use arkavo_protocol::classification_evidence::{
    ClassificationEvidence, Confidence, LabelFinding, TierReport,
};
use arkavo_protocol::data_classification::{
    ClassifiedDatum, DataCategory, DatumType, DlpAction, DlpPolicy, SensitivityLevel,
};
use arkavo_test_macros::spec;

fn datum(datum_type: DatumType, text: &str) -> ClassifiedDatum {
    ClassifiedDatum {
        datum_type,
        position: (0, text.len()),
        matched_text: text.into(),
    }
}

/// SENT-001: today the detector *is* the decision. Documents current behavior.
#[spec("SENT-001")]
#[test]
fn dlp_policy_returns_an_authorization_verdict() {
    let policy = DlpPolicy::strict();

    let action = policy.evaluate(&datum(DatumType::ApiKey, fake_api_key().as_str()));

    assert_eq!(action, DlpAction::Block);
}

/// SENT-001: the sentinel labels and the policy decision point authorizes, so
/// the classifier's return value is evidence rather than a verdict. Nothing a
/// tier can return names a disposition, which is what makes it structurally
/// impossible for a classifier to release or deny on its own authority.
#[spec("SENT-001")]
#[test]
fn classification_yields_labels_for_a_policy_decision_point() {
    let report = TierReport::matched(
        "reference-index",
        "1+1.0.0",
        vec![LabelFinding::new(
            DataCategory::Credentials,
            SensitivityLevel::Restricted,
            Confidence::CERTAIN,
            "12/40 shingles matched",
        )],
    );

    let finding = report.findings().first().expect("a label");
    assert_eq!(finding.category, DataCategory::Credentials);
    assert_eq!(finding.sensitivity, SensitivityLevel::Restricted);
    // And no disposition is reachable from it: there is no verdict to read out.
    let rendered = format!("{report:?}");
    assert!(
        !rendered.contains("Allow") && !rendered.contains("Block"),
        "SENT-001: a tier report must not carry a disposition: {rendered}"
    );
}

/// SENT-002: evidence carries calibrated confidence plus the detector and
/// taxonomy versions that produced it, and the source family when a reference
/// tier contributed — everything an auditor needs to reconstruct the call.
#[spec("SENT-002")]
#[test]
fn evidence_carries_confidence_and_both_versions() {
    let evidence = ClassificationEvidence::new("1.0.0").with_tier(TierReport::matched(
        "reference-index",
        "1+1.0.0",
        vec![
            LabelFinding::new(
                DataCategory::Pii,
                SensitivityLevel::Restricted,
                Confidence::new(0.87),
                "national identity number",
            )
            .from_family("hr-records")
            .at(4, 15),
        ],
    ));

    assert_eq!(evidence.taxonomy_version, "1.0.0");
    let tier = evidence.tiers.first().expect("a consulted tier");
    assert_eq!(tier.version, "1+1.0.0", "the detector version is recorded");
    let finding = tier.findings().first().expect("a label");
    assert_eq!(finding.confidence, Confidence::new(0.87));
    assert_eq!(finding.source_family.as_deref(), Some("hr-records"));
    assert_eq!(finding.span, Some((4, 15)));
    assert!(
        !finding.signal.is_empty(),
        "the signal that fired is recorded"
    );
}

/// SENT-002 edge case: a tier that contributed no signal is recorded as
/// consulted with no match, never omitted. Evidence that shrinks when a tier
/// finds nothing cannot be told apart from evidence that shrinks when a tier
/// stops running.
#[spec("SENT-002")]
#[test]
fn a_tier_that_found_nothing_is_still_recorded() {
    let evidence = ClassificationEvidence::new("1.0.0")
        .with_tier(TierReport::matched("pattern", "1", Vec::new()))
        .with_tier(TierReport::unavailable("near-duplicate", "1", "not loaded"));

    assert_eq!(evidence.tiers.len(), 2);
    // And the two are distinguishable: one looked and found nothing, the other
    // did not look.
    assert!(!evidence.tiers[0].is_unavailable());
    assert!(evidence.tiers[1].is_unavailable());
    assert!(evidence.has_gap());
}

/// SENT-003: the merge is evaluated over the whole payload, not per matched
/// datum. Evaluating one datum at a time is what let the low-sensitivity half
/// of a payload answer for the credential sitting in the same buffer.
#[spec("SENT-003")]
#[test]
fn a_payload_is_evaluated_at_the_level_of_the_highest_label_in_it() {
    let evidence = ClassificationEvidence::new("1.0.0").with_tier(TierReport::matched(
        "pattern",
        "1",
        vec![
            LabelFinding::new(
                DataCategory::Pii,
                SensitivityLevel::Internal,
                Confidence::CERTAIN,
                "Email",
            ),
            LabelFinding::new(
                DataCategory::Credentials,
                SensitivityLevel::Restricted,
                Confidence::CERTAIN,
                "ApiKey",
            ),
        ],
    ));

    assert_eq!(
        evidence.sensitivity_at(Confidence::new(0.5)),
        Some(SensitivityLevel::Restricted),
        "SENT-003: a payload holding a credential and an email is a credential payload"
    );
    let categories = evidence.categories_at(Confidence::new(0.5));
    assert!(categories.contains(&DataCategory::Pii));
    assert!(categories.contains(&DataCategory::Credentials));
}

/// SENT-003: the fused per-datum surface is still in place beside the evidence
/// contract, and still answers about one datum at a time. Documents what the
/// policy layer must not be built on.
#[spec("SENT-003")]
#[test]
fn the_fused_per_datum_surface_still_answers_about_one_datum() {
    let policy = DlpPolicy::strict();

    assert_eq!(
        policy.evaluate(&datum(DatumType::ApiKey, fake_api_key().as_str())),
        DlpAction::Block
    );
    assert_ne!(
        policy.evaluate(&datum(DatumType::Email, "person@example.com")),
        DlpAction::Block,
        "per-datum evaluation cannot see the rest of the payload"
    );
}

/// SENT-003: `SensitivityLevel` is ordered, which is what a monotonic union
/// needs, but nothing in the crate performs the union. Documents the ordering
/// the merge will rely on.
#[spec("SENT-003")]
#[test]
fn sensitivity_levels_are_ordered_for_a_future_union() {
    assert!(SensitivityLevel::Restricted > SensitivityLevel::Confidential);
    assert!(SensitivityLevel::Confidential > SensitivityLevel::Internal);
    assert!(SensitivityLevel::Internal > SensitivityLevel::Public);
}

/// SENT-013: an unavailable detector must hold content, not release it. A hold
/// is a third answer, distinct from both terminal ones: the content is neither
/// released nor refused while the question is unresolved.
#[spec("SENT-013")]
#[test]
fn dlp_action_has_a_hold_disposition() {
    let dispositions = [
        DlpAction::Allow,
        DlpAction::Block,
        DlpAction::Redact,
        DlpAction::Hold,
    ]
    .iter()
    .map(|a| format!("{a:?}"))
    .collect::<Vec<_>>()
    .join(",");

    assert!(dispositions.contains("Hold"), "{dispositions}");
    // A hold must not be either terminal answer wearing a different name.
    assert_ne!(DlpAction::Hold, DlpAction::Allow);
    assert_ne!(DlpAction::Hold, DlpAction::Block);
}

/// SENT-013: holding is not the same as refusing, and a caller that can only
/// distinguish allow from block will treat a hold as one of them. The
/// disposition has to survive a round trip through the wire form.
#[spec("SENT-013")]
#[test]
fn a_hold_survives_serialization_as_itself() {
    let json = serde_json::to_string(&DlpAction::Hold).expect("serialize");

    assert_eq!(json, r#""hold""#);
    assert_eq!(
        serde_json::from_str::<DlpAction>(&json).expect("deserialize"),
        DlpAction::Hold
    );
}

/// Builds a credential-shaped string at run time.
///
/// Generated rather than written down: a literal that matches a secret pattern
/// trips scanners on every clone of this repo, and a scanner that cries wolf on
/// fixtures is one people learn to ignore. The pieces are inert separately, and
/// the value is deterministic so a failure stays reproducible.
fn fake_api_key() -> String {
    let prefix: String = ['s', 'k'].iter().collect();
    let body: String = (0..24)
        .map(|i| char::from(b'a' + ((i * 7 + 3) % 26) as u8))
        .collect();
    format!("{prefix}-{body}")
}
