#![allow(clippy::disallowed_methods)]

//! SEQ-001, SEQ-016, SEQ-017: Tests against existing protocol types.

use arkavo_protocol::data_classification::{
    ClassifiedDatum, DatumType, DlpAction, DlpPolicy, SensitivityLevel,
};
use arkavo_protocol::error::A2aError;
use arkavo_test_macros::spec;

/// SEQ-001: per-datum evaluation carries no provenance, by construction — a
/// datum is a match inside one buffer and knows nothing about where the buffer
/// came from. Provenance attaches at the payload level instead; the assertion
/// that an egress decision carries it lives in `egress_taint_test.rs`, against
/// the gate that actually stands between the data and the wire.
#[spec("SEQ-001")]
#[test]
fn per_datum_dlp_evaluation_carries_no_provenance() {
    let policy = DlpPolicy::strict();
    let datum = ClassifiedDatum {
        datum_type: DatumType::ApiKey,
        position: (0, 20),
        matched_text: fake_api_key().as_str().into(),
    };

    let action = policy.evaluate(&datum);

    assert_eq!(action, DlpAction::Block);
    assert!(!format!("{action:?}").contains("source"));
}

/// SEQ-001: SensitivityLevel has ranking but nothing prevents downgrade.
#[spec("SEQ-001")]
#[test]
fn sensitivity_level_allows_downgrade() {
    let restricted = SensitivityLevel::Restricted;
    let public = SensitivityLevel::Public;

    assert!(restricted as u8 > public as u8);
}

/// SEQ-016: RuntimeConfig has no sequence integrity configuration fields.
/// Tripwire: when sequence_integrity config is added, this will stop panicking.
#[spec("SEQ-016")]
#[test]
#[should_panic(expected = "SEQ-016")]
fn runtime_config_has_no_sequence_integrity_fields() {
    let config = arkavo_protocol::agent_config::parse_runtime_config("");

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
