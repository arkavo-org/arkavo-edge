//! SENT-001, SENT-002, SENT-003, SENT-013: tripwires against the DLP surface
//! the sentinel will replace.
//!
//! `DlpPolicy` today fuses detection and authorization: it looks at one datum
//! and answers Allow or Block. The sentinel splits those apart — it produces
//! evidence, the policy decision point authorizes — so every test here pins
//! down a property of the current fused design that has to change.

use arkavo_protocol::data_classification::{
    ClassifiedDatum, DatumType, DlpAction, DlpPolicy, SensitivityLevel,
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
/// the classifier's return value must be evidence rather than a verdict.
/// Tripwire: flips when detection returns labels instead of Allow/Block.
#[spec("SENT-001")]
#[test]
#[should_panic(expected = "SENT-001")]
fn classification_result_is_not_separable_from_the_verdict() {
    let policy = DlpPolicy::strict();

    let action = policy.evaluate(&datum(DatumType::ApiKey, fake_api_key().as_str()));
    let action_str = format!("{action:?}");

    assert!(
        action_str.contains("labels"),
        "SENT-001: classification should yield labels for a policy decision point, \
         but evaluate returned the verdict itself: {action_str}"
    );
}

/// SENT-002: evidence must carry calibrated confidence plus the detector and
/// taxonomy versions that produced it, so an auditor can reconstruct the call.
/// Tripwire: flips when the evidence contract lands.
#[spec("SENT-002")]
#[test]
#[should_panic(expected = "SENT-002")]
fn classified_datum_carries_no_calibrated_evidence() {
    let classified = datum(DatumType::SocialSecurityNumber, fake_ssn().as_str());
    let evidence = format!("{classified:?}");

    assert!(
        evidence.contains("calibrated_confidence")
            && evidence.contains("detector_version")
            && evidence.contains("taxonomy_version"),
        "SENT-002: evidence should carry calibrated confidence and detector \
         plus taxonomy versions, but the classification is: {evidence}"
    );
}

/// SENT-003: labels merge by monotonic union over the whole payload. Evaluating
/// one datum at a time lets the low-sensitivity half of a payload answer Allow
/// while a credential sits in the same buffer.
/// Tripwire: flips when evaluation takes a payload taint set rather than a datum.
#[spec("SENT-003")]
#[test]
#[should_panic(expected = "SENT-003")]
fn per_datum_evaluation_ignores_the_rest_of_the_payload() {
    let policy = DlpPolicy::strict();
    let payload_contains_a_credential = datum(DatumType::ApiKey, fake_api_key().as_str());
    let same_payload_also_contains = datum(DatumType::Email, "person@example.com");

    assert_eq!(
        policy.evaluate(&payload_contains_a_credential),
        DlpAction::Block
    );
    let action = policy.evaluate(&same_payload_also_contains);

    assert_eq!(
        action,
        DlpAction::Block,
        "SENT-003: the whole payload carries the credential's classification, \
         but per-datum evaluation answered {action:?} for the email in it"
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

/// A national-identity-number-shaped string, assembled at run time for the same
/// reason as [`fake_api_key`].
fn fake_ssn() -> String {
    format!("{:03}-{:02}-{:04}", 123, 45, 6789)
}
