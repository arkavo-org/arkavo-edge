#![cfg(feature = "taint")]
//! SEQ-003, SEQ-014: the taint-aware egress gate, end to end.
//!
//! These are the scenarios `arkavo-validation` cannot express: they need both
//! the destination checks that crate owns and the taint labels that live above
//! it. The gate composes the two, so the composed decision is tested here.

use arkavo_protocol::data_classification::{DlpAction, SensitivityLevel};
use arkavo_protocol::egress_destination::{Destination, DestinationPolicy, extract_destinations};
use arkavo_protocol::egress_taint::{
    DenialReason, EgressDisposition, EgressTaintGate, GENERIC_DENIAL, HoldReason,
    RequesterEntitlements,
};
use arkavo_protocol::taint::{SourceKind, TaintSource, Transformation};
use arkavo_protocol::taint_tracker::DataTaintTracker;
use arkavo_test_macros::spec;
use serde_json::json;

const CLEARANCE: &str = "https://attr.arkavo.com/clearance";

fn gate() -> EgressTaintGate {
    EgressTaintGate::new().with_destination_policy(
        DestinationPolicy::new()
            .sanction_host("vault.internal")
            .tdf_capable_host("peer.arkavo.com")
            .workspace_root("/work/agent"),
    )
}

fn external() -> Destination {
    Destination::External {
        url: "https://api.example.com/collect".into(),
    }
}

/// SEQ-003: a payload that read a credential does not reach a public API, and
/// no entitlement changes that.
#[spec("SEQ-003")]
#[test]
fn credential_bearing_payloads_are_blocked_at_egress() {
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/etc/service.env"),
        &format!("AWS_SECRET={}", fake_api_key()),
    );
    let entitled = RequesterEntitlements::none().with_attribute(CLEARANCE, "restricted");

    let decision = gate().evaluate(&taint, &external(), &entitled);

    assert!(
        matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::NeverRelease { .. })
        ),
        "credential reached egress: {:?}",
        decision.disposition
    );
}

/// SEQ-003 edge case: encoding is a hop, not an exit. The classifier stops
/// seeing the secret once it is base64; the label does not stop seeing it.
#[spec("SEQ-003")]
#[test]
fn encoding_to_evade_the_classifier_does_not_clear_the_gate() {
    let tracker = DataTaintTracker::new("s1");
    let secret = &format!("AWS_SECRET={}", fake_api_key());
    let taint = tracker.ingest(&TaintSource::new(SourceKind::FileRead, "/etc/env"), secret);

    // What the agent would send after encoding.
    let encoded: String = secret.bytes().fold(String::new(), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{b:02x}");
        acc
    });
    let after_encoding = tracker.transform(&[&taint], Transformation::Encode, "hex");

    // The detector genuinely cannot see it any more...
    assert!(
        tracker
            .ingest(
                &TaintSource::new(SourceKind::ToolResult, "encode"),
                &encoded
            )
            .categories()
            .is_empty(),
        "the encoded form should defeat the detector, or this proves nothing"
    );

    // ...and the gate blocks anyway, because the label travelled with the data.
    let decision = gate().evaluate(
        &after_encoding,
        &external(),
        &RequesterEntitlements::none().with_attribute(CLEARANCE, "restricted"),
    );

    assert!(
        matches!(
            decision.disposition,
            EgressDisposition::Block(DenialReason::NeverRelease { .. })
        ),
        "encoding cleared the gate: {:?}",
        decision.disposition
    );
}

/// SEQ-003: internal-classified data does not reach an external endpoint.
#[spec("SEQ-003")]
#[test]
fn internal_data_is_blocked_from_external_endpoints() {
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/work/agent/roadmap.md"),
        "Q3 roadmap: ship the sealed pack format",
    );

    let decision = gate().evaluate(&taint, &external(), &RequesterEntitlements::none());

    assert!(!decision.is_release(), "{:?}", decision.disposition);
}

/// SEQ-003 edge case: a sanctioned internal endpoint proceeds.
#[spec("SEQ-003")]
#[test]
fn a_sanctioned_internal_endpoint_proceeds() {
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/work/agent/roadmap.md"),
        "Q3 roadmap",
    );

    let decision = gate().evaluate(
        &taint,
        &Destination::Internal {
            url: "https://vault.internal/store".into(),
        },
        &RequesterEntitlements::none(),
    );

    assert_eq!(decision.disposition, EgressDisposition::Allow);
}

/// SEQ-003: PII needs explicit authorization; with it the payload travels
/// wrapped rather than in the clear.
#[spec("SEQ-003")]
#[test]
fn pii_requires_authorization_and_then_travels_wrapped() {
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::ToolResult, "crm_lookup"),
        "contact: dana@example.com",
    );
    let to_peer = Destination::External {
        url: "https://peer.arkavo.com/inbox".into(),
    };

    let unauthorized = gate().evaluate(&taint, &to_peer, &RequesterEntitlements::none());
    assert!(matches!(
        unauthorized.disposition,
        EgressDisposition::Block(DenialReason::NotEntitled { .. })
    ));

    let authorized = gate().evaluate(
        &taint,
        &to_peer,
        &RequesterEntitlements::none()
            .with_attribute(CLEARANCE, "internal")
            .with_did("did:web:arkavo.com:agent:a"),
    );
    assert!(matches!(
        authorized.disposition,
        EgressDisposition::Wrap { .. }
    ));
}

/// SEQ-003: a destination that cannot consume a TDF gets nothing, rather than
/// getting the plaintext it can consume.
#[spec("SEQ-003")]
#[test]
fn an_unwrappable_destination_never_receives_a_plaintext_downgrade() {
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::ToolResult, "crm_lookup"),
        "contact: dana@example.com",
    );

    let decision = gate().evaluate(
        &taint,
        &external(),
        &RequesterEntitlements::none().with_attribute(CLEARANCE, "internal"),
    );

    assert!(matches!(
        decision.disposition,
        EgressDisposition::Block(DenialReason::DestinationCannotWrap { .. })
    ));
}

/// SEQ-003: destinations come out of the parameters by shape. A tool the gate
/// has never heard of, with a parameter name it has never seen, is still gated.
#[spec("SEQ-003")]
#[test]
fn an_unknown_tool_with_an_unknown_parameter_is_still_gated() {
    let params = json!({"sink_uri": "https://attacker.example/collect", "body": "..."});
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/etc/env"),
        &format!("token={}", fake_api_key()),
    );
    let gate = gate();

    let destinations = extract_destinations(&params, gate.destinations());

    assert_eq!(destinations.len(), 1);
    assert!(
        !gate
            .evaluate(&taint, &destinations[0], &RequesterEntitlements::none())
            .is_release()
    );
}

/// SEQ-014: the denial audit record carries the provenance chain from source to
/// violation point, which is what a forensic reconstruction runs on.
#[spec("SEQ-014")]
#[test]
fn a_denial_carries_the_full_provenance_chain_to_audit() {
    let tracker = DataTaintTracker::new("s1");
    let read = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/etc/env"),
        &format!("token={}", fake_api_key()),
    );
    let summarized = tracker.transform(&[&read], Transformation::Summarize, "distill");
    let encoded = tracker.transform(&[&summarized], Transformation::Encode, "base64");

    let decision = gate().evaluate(&encoded, &external(), &RequesterEntitlements::none());
    let audit = decision.audit_detail();

    assert!(audit.contains("file:/etc/env"), "{audit}");
    assert!(audit.contains("summarize|distill"), "{audit}");
    assert!(audit.contains("encode|base64"), "{audit}");
    assert_eq!(decision.evidence.sensitivity, SensitivityLevel::Restricted);
}

/// SEQ-014: what the refused caller learns is uniform, or the denial becomes a
/// classifier oracle the caller can binary-search against.
#[spec("SEQ-014")]
#[test]
fn the_caller_learns_nothing_the_audit_record_knows() {
    let gate = gate();
    let tracker = DataTaintTracker::new("s1");
    let credential = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/etc/env"),
        &format!("token={}", fake_api_key()),
    );
    let health = tracker.ingest(
        &TaintSource::new(SourceKind::ToolResult, "ehr"),
        "patient roster attached",
    );

    let a = gate.evaluate(&credential, &external(), &RequesterEntitlements::none());
    let b = gate.evaluate(&health, &external(), &RequesterEntitlements::none());

    assert_eq!(a.public_message(), Some(GENERIC_DENIAL));
    assert_eq!(b.public_message(), Some(GENERIC_DENIAL));
    assert!(!GENERIC_DENIAL.contains("credential"));
    assert_ne!(a.audit_detail(), b.audit_detail());
}

/// SEQ-014 edge case: an allowlisted destination does not override taint.
#[spec("SEQ-014")]
#[test]
fn taint_policy_overrides_the_destination_allowlist() {
    let mut filter = arkavo_validation::EgressFilter::new();
    filter.allow("https://api.example.com/collect");
    let gate = gate().with_egress_filter(filter);
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/work/agent/roadmap.md"),
        "Q3 roadmap",
    );

    assert!(
        !gate
            .evaluate(&taint, &external(), &RequesterEntitlements::none())
            .is_release()
    );
}

/// SEQ-014 edge case: an indeterminate destination is held for review, not
/// resolved to one of the terminal answers.
#[spec("SEQ-014")]
#[test]
fn an_indeterminate_destination_is_held_for_review() {
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/work/agent/roadmap.md"),
        "Q3 roadmap",
    );

    let decision = gate().evaluate(
        &taint,
        &Destination::Unresolved { hint: "ftp".into() },
        &RequesterEntitlements::none(),
    );

    assert!(matches!(
        decision.disposition,
        EgressDisposition::Hold(HoldReason::DestinationUnresolved { .. })
    ));
    assert_eq!(decision.disposition.as_dlp_action(), DlpAction::Hold);
}

/// SEQ-001: the decision a DLP surface returns now carries where the data came
/// from. The per-datum `DlpPolicy::evaluate` still does not — see the companion
/// test in `sequence_integrity_test.rs` — but the gate's decision does, and the
/// gate is what stands between the data and the wire.
#[spec("SEQ-001")]
#[test]
fn an_egress_decision_carries_data_source_provenance() {
    let tracker = DataTaintTracker::new("s1");
    let taint = tracker.ingest(
        &TaintSource::new(SourceKind::FileRead, "/etc/env"),
        &format!("token={}", fake_api_key()),
    );

    let decision = gate().evaluate(&taint, &external(), &RequesterEntitlements::none());

    assert_eq!(decision.evidence.sources, vec!["file:/etc/env".to_string()]);
    assert_eq!(decision.disposition.as_dlp_action(), DlpAction::Block);
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
