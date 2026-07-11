//! Pure SSE helpers for the xAI Responses stream parser.
//!
//! Extracted so partial-line buffering and single-terminal-done rules are
//! unit-tested against the same code the stream task runs.

use crate::StreamResponse;
use crate::provider::InferenceTiming;
use serde_json::Value;

use super::types::{ResponsesUsage, timing_from_usage};

/// Outcome of handling one complete `data: ...` SSE line.
#[derive(Debug)]
pub(super) enum SseAction {
    /// Emit a stream chunk (delta or terminal done).
    Emit(StreamResponse),
    /// Stream failed; emit error and stop. Payload is the provider error message.
    Fail(String),
    /// Stream finished after a terminal signal (e.g. `[DONE]` after done already sent).
    Finished,
    /// Ignore this event type / unparseable payload.
    Ignore,
}

/// Drain complete (newline-terminated) lines from `buffer`, leaving any partial
/// trailing segment in place. Returns the drained complete text.
pub(super) fn drain_complete_sse_lines(buffer: &mut String) -> Option<String> {
    let last_newline = buffer.rfind('\n')?;
    Some(buffer.drain(..=last_newline).collect())
}

/// Parse one complete SSE `data:` payload into a stream action.
///
/// `terminal_sent` tracks whether a `done: true` chunk was already emitted so
/// `response.completed` + trailing `[DONE]` only signal once.
pub(super) fn handle_sse_data_line(
    data: &str,
    terminal_sent: bool,
    on_response_id: &mut dyn FnMut(String),
) -> SseAction {
    if data == "[DONE]" {
        if terminal_sent {
            return SseAction::Finished;
        }
        return SseAction::Emit(StreamResponse {
            content: String::new(),
            reasoning_content: None,
            done: true,
            inference_timing: None,
        });
    }

    let Ok(event) = serde_json::from_str::<Value>(data) else {
        return SseAction::Ignore;
    };
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                SseAction::Emit(StreamResponse {
                    content: delta.to_string(),
                    reasoning_content: None,
                    done: false,
                    inference_timing: None,
                })
            } else {
                SseAction::Ignore
            }
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                SseAction::Emit(StreamResponse {
                    content: String::new(),
                    reasoning_content: Some(delta.to_string()),
                    done: false,
                    inference_timing: None,
                })
            } else {
                SseAction::Ignore
            }
        }
        "response.completed" => {
            if let Some(id) = event.pointer("/response/id").and_then(Value::as_str) {
                on_response_id(id.to_string());
            }
            if terminal_sent {
                return SseAction::Ignore;
            }
            let timing: Option<InferenceTiming> = event.pointer("/response/usage").and_then(|u| {
                let usage: ResponsesUsage = serde_json::from_value(u.clone()).ok()?;
                Some(timing_from_usage(&usage))
            });
            SseAction::Emit(StreamResponse {
                content: String::new(),
                reasoning_content: None,
                done: true,
                inference_timing: timing,
            })
        }
        "response.failed" => {
            let msg = event
                .pointer("/response/error")
                .map(|e| e.to_string())
                .unwrap_or_else(|| "response.failed".to_string());
            SseAction::Fail(msg)
        }
        _ => SseAction::Ignore,
    }
}

/// Whether emitting this action should mark the terminal done as sent.
pub(super) fn action_sets_terminal(action: &SseAction) -> bool {
    matches!(action, SseAction::Emit(StreamResponse { done: true, .. }))
}

/// After `response.completed` we keep reading so a trailing `[DONE]` can be
/// absorbed without a second emit; after bare `[DONE]` we stop; after Fail we stop.
pub(super) fn should_stop_after(action: &SseAction, data_line: &str) -> bool {
    match action {
        SseAction::Fail(_) | SseAction::Finished => true,
        SseAction::Emit(r) if r.done && data_line == "[DONE]" => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_keeps_partial_line_until_newline() {
        let mut buffer =
            String::from("data: {\"type\":\"response.output_text.delta\",\"delta\":\"a\"}");
        assert!(
            drain_complete_sse_lines(&mut buffer).is_none(),
            "unterminated line must not look complete"
        );
        buffer.push_str("\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"b\"}\n");
        let complete = drain_complete_sse_lines(&mut buffer).expect("complete lines present");
        let lines: Vec<&str> = complete
            .lines()
            .filter(|l| l.starts_with("data: "))
            .collect();
        assert_eq!(lines.len(), 2, "both complete data lines should parse");
        assert!(
            buffer.is_empty(),
            "no partial remainder after trailing newline"
        );

        buffer.push_str("data: {\"type\":\"response.completed\"");
        assert!(
            drain_complete_sse_lines(&mut buffer).is_none(),
            "partial completed event must remain buffered"
        );
        buffer.push_str("}\n");
        let complete = drain_complete_sse_lines(&mut buffer).unwrap();
        assert!(complete.contains("response.completed"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn terminal_done_emitted_only_once() {
        let mut terminal_sent = false;
        let mut response_ids = Vec::new();
        let mut emitted = 0usize;

        let completed = r#"{"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":1,"output_tokens":2}}}"#;
        for data in [completed, "[DONE]", "[DONE]"] {
            let action = handle_sse_data_line(data, terminal_sent, &mut |id| {
                response_ids.push(id);
            });
            if action_sets_terminal(&action) {
                emitted += 1;
                terminal_sent = true;
            }
            if should_stop_after(&action, data) {
                break;
            }
        }

        assert_eq!(emitted, 1, "only one terminal done signal");
        assert!(terminal_sent);
        assert_eq!(response_ids, vec!["resp_1".to_string()]);
    }

    #[test]
    fn output_text_delta_emits_content() {
        let data = r#"{"type":"response.output_text.delta","delta":"hi"}"#;
        let action = handle_sse_data_line(data, false, &mut |_| {});
        match action {
            SseAction::Emit(r) => {
                assert_eq!(r.content, "hi");
                assert!(!r.done);
            }
            other => panic!("expected Emit, got {other:?}"),
        }
    }

    #[test]
    fn bare_done_is_terminal() {
        let action = handle_sse_data_line("[DONE]", false, &mut |_| {});
        assert!(matches!(
            action,
            SseAction::Emit(StreamResponse { done: true, .. })
        ));
        assert!(should_stop_after(&action, "[DONE]"));
    }
}
