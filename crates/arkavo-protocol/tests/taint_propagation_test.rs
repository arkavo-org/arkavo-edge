#![cfg(feature = "taint")]

//! SEQ-001, SEQ-002: taint labelling and monotonic propagation.
//!
//! These cover the substrate. The scenarios stay `wip` until the tracker is
//! wired at the tool-dispatch and A2A seams, which is Phase 2 work; what is
//! asserted here is that the labels themselves behave.

use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_protocol::taint::{SourceKind, TaintLabel, TaintSet, TaintSource, Transformation};
use arkavo_protocol::taint_inference::{ClassificationInferencer, RegexInferencer};
use arkavo_test_macros::spec;

/// The payload every propagation test carries. A function rather than a const
/// because the credential inside it is built at run time.
fn secret() -> String {
    format!("deploy key {} rotate monthly", fake_api_key())
}

fn labelled(source: &TaintSource, text: &str, floor: SensitivityLevel) -> TaintSet {
    let found = RegexInferencer::new().infer(text);
    TaintSet::from_label(TaintLabel::from_classifications(
        source.source_id(),
        &found,
        floor,
    ))
}

/// SEQ-001: the label names where the data came from, not just how sensitive
/// it is — a gate that cannot say "from where" cannot produce a provenance
/// chain for the audit record.
#[spec("SEQ-001")]
#[test]
fn ingested_data_carries_its_source_identifier() {
    let source = TaintSource::new(SourceKind::FileRead, "/etc/deploy.env");

    let set = labelled(&source, secret().as_str(), SensitivityLevel::Internal);

    assert_eq!(
        set.source_ids().collect::<Vec<_>>(),
        vec!["file:/etc/deploy.env"]
    );
}

/// SEQ-001: classification level is inferred from content, not assumed from
/// the source.
#[spec("SEQ-001")]
#[test]
fn classification_is_inferred_from_content() {
    let source = TaintSource::new(SourceKind::ToolResult, "read_file");

    let set = labelled(&source, secret().as_str(), SensitivityLevel::Public);

    assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);
    assert!(set.contains_category(DataCategory::Credentials));
}

/// SEQ-001 edge case: an ambiguous source classifies conservatively. A scan
/// that found nothing is not evidence that there is nothing to find.
#[spec("SEQ-001")]
#[test]
fn an_ambiguous_source_classifies_conservatively() {
    let source = TaintSource::new(SourceKind::Unknown, "unattributed-buffer");

    let set = labelled(
        &source,
        "nothing recognizable here",
        SensitivityLevel::Internal,
    );

    assert_eq!(set.sensitivity(), SensitivityLevel::Internal);
}

/// SEQ-001 edge case: merging mixed sensitivities propagates the highest.
#[spec("SEQ-001")]
#[test]
fn merging_mixed_sensitivity_propagates_the_highest() {
    let public = TaintSet::from_label(TaintLabel::new(
        "tool:docs",
        [DataCategory::Public],
        SensitivityLevel::Public,
    ));
    let restricted = TaintSet::from_label(TaintLabel::new(
        "tool:vault",
        [DataCategory::Credentials],
        SensitivityLevel::Restricted,
    ));

    let merged = public.union(&restricted);

    assert_eq!(merged.sensitivity(), SensitivityLevel::Restricted);
    assert!(merged.contains_category(DataCategory::Public));
    assert!(merged.contains_category(DataCategory::Credentials));
}

/// SEQ-002: output inherits every input's taint.
#[spec("SEQ-002")]
#[test]
fn output_inherits_taint_from_all_inputs() {
    let a = TaintSet::from_label(TaintLabel::new(
        "file:a",
        [DataCategory::Pii],
        SensitivityLevel::Internal,
    ));
    let b = TaintSet::from_label(TaintLabel::new(
        "file:b",
        [DataCategory::Financial],
        SensitivityLevel::Confidential,
    ));

    let combined = a.union(&b).transformed(Transformation::Merge, "concat");

    assert_eq!(combined.len(), 2);
    assert_eq!(combined.sensitivity(), SensitivityLevel::Confidential);
}

/// SEQ-002: the transformation itself is recorded, so an auditor can replay
/// how a buffer reached the sink it reached.
#[spec("SEQ-002")]
#[test]
fn transformation_type_is_recorded_in_the_provenance_chain() {
    let set = TaintSet::from_label(TaintLabel::new(
        "tool:vault",
        [DataCategory::Credentials],
        SensitivityLevel::Restricted,
    ))
    .transformed(Transformation::Summarize, "gemma-e2b")
    .transformed(Transformation::Encode, "base64");

    let hops = &set.label_for("tool:vault").expect("label survives").hops;

    assert_eq!(hops.len(), 2);
    assert_eq!(hops[0].transformation, Transformation::Summarize);
    assert_eq!(hops[0].detail, "gemma-e2b");
    assert_eq!(hops[1].transformation, Transformation::Encode);
}

/// SEQ-002: encoding does not strip taint. This is the load-bearing case —
/// after encoding, the detector genuinely cannot see the secret any more, and
/// the label is the only thing standing between it and an egress path.
#[spec("SEQ-002")]
#[test]
fn encoding_blinds_the_detector_but_not_the_label() {
    let source = TaintSource::new(SourceKind::ToolResult, "read_secrets");
    let set = labelled(&source, secret().as_str(), SensitivityLevel::Internal);
    assert_eq!(set.sensitivity(), SensitivityLevel::Restricted);

    let encoded: String = secret().bytes().fold(String::new(), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{b:02x}");
        acc
    });
    assert!(
        RegexInferencer::new().infer(&encoded).is_empty(),
        "the encoded form must defeat the detector, or this proves nothing"
    );

    let carried = set.transformed(Transformation::Encode, "hex");

    assert_eq!(carried.sensitivity(), SensitivityLevel::Restricted);
    assert!(carried.contains_category(DataCategory::Credentials));
}

/// SEQ-002: the same holds for a structural re-encoding.
#[spec("SEQ-002")]
#[test]
fn json_wrapping_does_not_strip_taint() {
    let source = TaintSource::new(SourceKind::ToolResult, "read_secrets");
    let set = labelled(&source, secret().as_str(), SensitivityLevel::Internal);

    let wrapped = serde_json::json!({ "payload": secret() }).to_string();
    let carried = set.transformed(Transformation::Encode, "json");

    assert!(wrapped.contains("payload"));
    assert_eq!(carried.sensitivity(), SensitivityLevel::Restricted);
}

/// SEQ-002 edge case: a field extracted from a tainted record inherits the
/// parent's taint even when the field itself looks innocuous.
#[spec("SEQ-002")]
#[test]
fn an_extracted_field_inherits_the_parent_taint() {
    let source = TaintSource::new(SourceKind::ToolResult, "read_record");
    let record = labelled(&source, secret().as_str(), SensitivityLevel::Internal);

    let field = record.transformed(Transformation::Extract, "record.comment");

    assert_eq!(field.sensitivity(), SensitivityLevel::Restricted);
}

/// SEQ-002: summarizing keeps the source classification. A shorter restatement
/// of a secret is still the secret.
#[spec("SEQ-002")]
#[test]
fn summarization_preserves_the_source_classification() {
    let source = TaintSource::new(SourceKind::FileRead, "/etc/deploy.env");
    let set = labelled(&source, secret().as_str(), SensitivityLevel::Internal);

    let summary = set.transformed(Transformation::Summarize, "gemma-e2b");

    assert_eq!(summary.sensitivity(), SensitivityLevel::Restricted);
    assert_eq!(
        summary.source_ids().collect::<Vec<_>>(),
        vec!["file:/etc/deploy.env"]
    );
}

/// SEQ-002: a transformation cannot be used to launder a label. Unioning a
/// deliberately clean set over a tainted one leaves the taint in place.
#[spec("SEQ-002")]
#[test]
fn a_transformation_cannot_lower_a_label() {
    let tainted = TaintSet::from_label(TaintLabel::new(
        "tool:vault",
        [DataCategory::Credentials],
        SensitivityLevel::Restricted,
    ));
    let claimed_public = TaintSet::from_label(TaintLabel::new(
        "tool:vault",
        [DataCategory::Public],
        SensitivityLevel::Public,
    ));

    let laundered = tainted
        .union(&claimed_public)
        .transformed(Transformation::Other, "rewrite");

    assert_eq!(laundered.sensitivity(), SensitivityLevel::Restricted);
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
