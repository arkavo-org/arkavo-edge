#![allow(clippy::disallowed_methods)]

//! SEQ-001, SEQ-016, SEQ-017: Tests against existing protocol types.

use arkavo_protocol::data_classification::{
    ClassifiedDatum, DatumType, DlpAction, DlpPolicy, SensitivityLevel,
};
use arkavo_protocol::error::A2aError;
use arkavo_test_macros::spec;

/// SEQ-001: DlpPolicy evaluates classified data but returns a simple Allow/Block.
/// No provenance chain, no source identifier, no transformation history.
#[spec("SEQ-001")]
#[test]
fn dlp_action_carries_no_provenance() {
    let policy = DlpPolicy::strict();
    let datum = ClassifiedDatum {
        datum_type: DatumType::ApiKey,
        position: (0, 20),
        matched_text: "sk-abc123".into(),
    };

    let action = policy.evaluate(&datum);
    assert_eq!(action, DlpAction::Block);

    // Correctly blocks — but action is just an enum (Allow/Block/Redact).
    // SEQ-001 requires: TaintLabel with source_id, classification, provenance_chain.
    let action_str = format!("{action:?}");
    assert!(
        action_str.contains("source"),
        "SEQ-001: DLP action should include data source provenance, \
         but current action is: {action_str}"
    );
}

/// SEQ-001: SensitivityLevel has ranking but nothing prevents downgrade.
#[spec("SEQ-001")]
#[test]
fn sensitivity_level_allows_downgrade() {
    let restricted = SensitivityLevel::Restricted;
    let public = SensitivityLevel::Public;

    assert!(restricted as u8 > public as u8);
    // SEQ-002 requires: transformations should never reduce classification.
}

/// SEQ-016: RuntimeConfig has no sequence integrity configuration fields.
#[spec("SEQ-016")]
#[test]
fn runtime_config_has_no_sequence_integrity_fields() {
    let config = arkavo_protocol::agent_config::parse_runtime_config("");

    // RuntimeConfig has: timeouts, pool connections, categorization_threshold
    // But no: enabled, taint_tracking, gate_thresholds, ledger_retention
    let config_str = format!("{config:?}");
    assert!(
        config_str.contains("sequence_integrity") || config_str.contains("taint"),
        "SEQ-016: RuntimeConfig should have sequence integrity fields, \
         but current fields are: {config_str}"
    );
}

/// SEQ-017: A2aError has no variant for sequence integrity errors.
#[spec("SEQ-017")]
#[test]
fn a2a_error_has_no_sequence_integrity_variant() {
    let err = A2aError::Protocol("sequence gap: expected 5, got 7".into());
    let err_str = format!("{err}");
    assert!(err_str.contains("sequence gap"));

    let code = err.to_json_rpc_code();
    assert_ne!(
        code, -32603,
        "SEQ-017: sequence errors should have specific codes, \
         not reuse internal error code {code}"
    );
}
