//! SEQ-015: Tests that Event struct supports sequence integrity evidence.

use arkavo_events::{Event, EventPayload, SequenceBaseline, TaintRecord};
use arkavo_test_macros::spec;

fn make_event(sequence: u64) -> Event {
    Event::new(
        "test-session".into(),
        sequence,
        "test-agent".into(),
        EventPayload::ToolCall {
            tool_name: "read_file".into(),
            parameters: serde_json::json!({}),
            tool_call_id: None,
        },
    )
}

fn taint() -> TaintRecord {
    TaintRecord {
        sensitivity: "restricted".into(),
        categories: vec!["credentials".into()],
        sources: vec!["file:/etc/creds".into()],
        provenance: vec![
            "file:/etc/creds|extract|read_file".into(),
            "file:/etc/creds|encode|base64".into(),
        ],
        truncated_hops: 0,
    }
}

/// SEQ-015: Events carry sequence numbers but nothing enforces monotonic ordering.
#[spec("SEQ-015")]
#[test]
fn events_allow_duplicate_sequence_numbers() {
    let event1 = make_event(5);
    let event2 = make_event(5);

    assert_eq!(event1.sequence, event2.sequence);
}

/// SEQ-015: metadata carries the taint chain, so a replay can follow data
/// across events without reconstructing it from each payload.
#[spec("SEQ-015")]
#[test]
fn event_metadata_carries_the_taint_chain() {
    let mut event = make_event(1);
    event.metadata.taint_chain = Some(taint());

    let serialized = serde_json::to_string(&event.metadata).unwrap();

    assert!(
        serialized.contains("taint_chain") && serialized.contains("file:/etc/creds"),
        "SEQ-015: metadata should carry the taint chain, but serialized as: {serialized}"
    );
}

/// SEQ-015: an untracked event says so by omission rather than by asserting a
/// chain it does not have. A reader must be able to tell the two apart.
#[spec("SEQ-015")]
#[test]
fn an_untracked_event_omits_the_chain_rather_than_claiming_an_empty_one() {
    let event = make_event(1);

    let serialized = serde_json::to_string(&event.metadata).unwrap();

    assert!(!serialized.contains("taint_chain"), "{serialized}");
}

/// SEQ-015: a violation is its own payload, carrying the action graph, the
/// taint chain to the violation point, and a baseline comparison when one
/// exists — enough to reconstruct the sequence forensically.
#[spec("SEQ-015")]
#[test]
fn sequence_violation_payload_carries_forensic_evidence() {
    let payload = EventPayload::SequenceViolation {
        violation_type: "egress_denied".into(),
        disposition: "block".into(),
        destination: "external:https://attacker.example/collect".into(),
        taint: taint(),
        action_graph: vec!["n0|read_file|a1b2|".into(), "n1|http_post|c3d4|n0".into()],
        baseline: Some(SequenceBaseline {
            expected: "read_file -> summarize".into(),
            actual: "read_file -> http_post".into(),
            derived_from: "session baseline".into(),
        }),
        correlation_id: Some("corr-7".into()),
    };

    let serialized = serde_json::to_string(&payload).unwrap();

    assert!(serialized.contains("action_graph"), "{serialized}");
    assert!(
        serialized.contains("file:/etc/creds|encode|base64"),
        "{serialized}"
    );
    assert!(
        serialized.contains("read_file -> http_post"),
        "{serialized}"
    );
    assert!(serialized.contains("corr-7"), "{serialized}");
}

/// SEQ-015: the violation is a distinct event type, so an audit sink can select
/// on it without parsing an error string.
#[spec("SEQ-015")]
#[test]
fn sequence_violation_has_its_own_event_type() {
    let event = Event::new(
        "s".into(),
        1,
        "a".into(),
        EventPayload::SequenceViolation {
            violation_type: "egress_held".into(),
            disposition: "hold".into(),
            destination: "unresolved:s3".into(),
            taint: TaintRecord::default(),
            action_graph: Vec::new(),
            baseline: None,
            correlation_id: None,
        },
    );

    assert_eq!(event.event_type(), "sequence_violation");
}

/// SEQ-015: a violation with no baseline says so rather than fabricating one.
/// Baseline infrastructure does not exist yet; the field must stay honest about
/// that rather than emitting a comparison against nothing.
#[spec("SEQ-015")]
#[test]
fn a_violation_without_a_baseline_omits_it() {
    let payload = EventPayload::SequenceViolation {
        violation_type: "egress_denied".into(),
        disposition: "block".into(),
        destination: "external:https://x/".into(),
        taint: TaintRecord::default(),
        action_graph: Vec::new(),
        baseline: None,
        correlation_id: None,
    };

    let serialized = serde_json::to_string(&payload).unwrap();

    assert!(!serialized.contains("baseline"), "{serialized}");
}
