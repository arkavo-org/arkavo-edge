//! SEQ-015: Tests that Event struct supports sequence integrity evidence.

use arkavo_events::{Event, EventPayload};
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

/// SEQ-015: Events carry sequence numbers but nothing enforces monotonic ordering.
#[spec("SEQ-015")]
#[test]
fn events_allow_duplicate_sequence_numbers() {
    let event1 = make_event(5);
    let event2 = make_event(5);

    assert_eq!(event1.sequence, event2.sequence);
    // SEQ-015: sequence numbers should be validated for gaps/duplicates
}

/// SEQ-015: EventMetadata has correlation_id but no taint_chain field.
#[spec("SEQ-015")]
#[test]
fn event_metadata_has_no_taint_chain() {
    let event = make_event(1);

    let serialized = serde_json::to_string(&event.metadata).unwrap();
    assert!(
        serialized.contains("taint"),
        "SEQ-015: EventMetadata should include taint chain field, \
         but current fields are: {serialized}"
    );
}

/// SEQ-015: Event payload has no variant for sequence integrity evidence.
#[spec("SEQ-015")]
#[test]
fn event_payload_has_no_sequence_violation_variant() {
    let error_payload = EventPayload::Error {
        error_type: "sequence_violation".into(),
        message: "sequence violation detected".into(),
        stack_trace: None,
        recoverable: None,
    };

    let serialized = serde_json::to_string(&error_payload).unwrap();
    assert!(
        serialized.contains("action_graph"),
        "SEQ-015: sequence violation events should include action graph, \
         but current Error payload is: {serialized}"
    );
}
