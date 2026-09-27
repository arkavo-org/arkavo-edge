use arkavo_llm::{Message, Role};

/// Returns how many messages after the initial system prompt may be summarized.
/// A retained tool output must keep its originating call and the entire batch.
pub(super) fn compactable_prefix(messages: &[Message], keep_recent: usize) -> usize {
    let mut start = messages.len().saturating_sub(keep_recent).max(1);
    start = start.min(messages.len());
    loop {
        let mut boundary = start;
        for message in &messages[start..] {
            if message.role != Role::Tool {
                continue;
            }
            let owner = messages[..start]
                .iter()
                .rposition(|candidate| {
                    candidate.role == Role::Assistant
                        && candidate
                            .tool_calls
                            .iter()
                            .any(|call| call.id == message.tool_call_id)
                })
                .or_else(|| {
                    messages[..start].iter().rposition(|candidate| {
                        candidate
                            .provider_state
                            .native_call_ids()
                            .any(|call_id| Some(call_id) == message.tool_call_id.as_deref())
                    })
                });
            if let Some(owner) = owner {
                boundary = boundary.min(owner);
            }
        }
        if boundary == start {
            return start.saturating_sub(1);
        }
        start = boundary;
    }
}

/// Model output can end anywhere in a UTF-8 scalar; summarize by characters.
pub(super) fn summary_line(message: &Message) -> String {
    format!(
        "[{:?}] {}",
        message.role,
        message.content.chars().take(500).collect::<String>()
    )
}

/// A byte-bounded excerpt of model or tool text, marked when it was cut.
///
/// Every one of these limits is a byte budget, and the text can end anywhere in
/// a UTF-8 scalar, so the cut has to land on a character boundary.
pub(super) fn preview(text: &str, max_bytes: usize) -> String {
    let kept = arkavo_llm::char_boundary_prefix(text, max_bytes);
    if kept.len() < text.len() {
        format!("{kept}...")
    } else {
        text.to_string()
    }
}

/// Close a loop whose last turn was a tool call rather than text.
///
/// `compute_response_quality("", ..)` returns 0.0, which pins the Thompson
/// Sampling average at 0% for models that answer purely through tools.
pub(super) fn tool_only_summary(steps: usize, last_result: Option<&str>) -> String {
    let last = last_result
        .map(|content| arkavo_llm::char_boundary_prefix(content, 200))
        .unwrap_or("ok");
    format!("Completed {steps} tool call(s). Last result: {last}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    fn batch(ids: &[&str]) -> Message {
        let mut message = Message::assistant("");
        message.provider_state = arkavo_llm::ProviderState::openai_responses(
            ids.iter()
                .map(|id| {
                    json!({
                        "type": "function_call", "call_id": id, "name": "read", "arguments": "{}"
                    })
                })
                .collect(),
        );
        message
    }

    #[spec("ASTRA-002")]
    #[test]
    fn compaction_keeps_arbitrary_native_batch_with_user_nudge() {
        let messages = vec![
            Message::system("instructions"),
            Message::user("old task"),
            Message::assistant("old answer"),
            batch(&["a", "b", "c"]),
            Message::tool_result("a", "a", "read"),
            Message::tool_result("b", "b", "read"),
            Message::tool_result("c", "c", "read"),
            Message::user("adjust strategy"),
        ];
        assert_eq!(compactable_prefix(&messages, 2), 2);
        let mut compacted = messages.clone();
        compacted.drain(1..=compactable_prefix(&messages, 2));
        assert_eq!(compacted[1].provider_state.native_call_ids().count(), 3);
        assert_eq!(compacted.last().unwrap().content, "adjust strategy");
    }

    #[spec("ASTRA-002")]
    #[test]
    fn compaction_drops_complete_old_batch_and_keeps_recent_exchange() {
        let messages = vec![
            Message::system("instructions"),
            batch(&["old"]),
            Message::tool_result("old", "old", "read"),
            Message::assistant("done"),
            Message::user("new task"),
            Message::assistant("new answer"),
        ];
        assert_eq!(compactable_prefix(&messages, 2), 3);
    }

    #[spec("ASTRA-002")]
    #[test]
    fn batch_at_beginning_cannot_be_split() {
        let messages = vec![
            Message::system("instructions"),
            batch(&["a", "b"]),
            Message::tool_result("a", "a", "read"),
            Message::tool_result("b", "b", "read"),
        ];
        assert_eq!(compactable_prefix(&messages, 2), 0);
    }

    #[test]
    fn summary_is_safe_for_multibyte_text() {
        let summary = summary_line(&Message::assistant("界".repeat(501)));
        assert_eq!(summary.matches('界').count(), 500);
    }
    /// conductor_tool_loop's raw-response eprintln (1000 bytes, no ellipsis).
    #[test]
    fn raw_response_preview_is_safe_for_multibyte_text() {
        let content = "界".repeat(400);
        assert_eq!(
            arkavo_llm::char_boundary_prefix(&content, 1000)
                .matches('界')
                .count(),
            333
        );
    }

    /// conductor_tool_loop's `debug!("LLM response content: ...")` (500 bytes).
    #[test]
    fn response_content_preview_is_safe_for_multibyte_text() {
        let excerpt = preview(&"界".repeat(400), 500);
        assert_eq!(excerpt.matches('界').count(), 166);
        assert!(excerpt.ends_with("..."));
        assert_eq!(preview("short", 500), "short");
    }

    /// conductor_parallel's condensed tool result (800 bytes).
    #[test]
    fn condensed_result_preview_is_safe_for_multibyte_text() {
        let excerpt = preview(&"界".repeat(400), 800);
        assert_eq!(excerpt.matches('界').count(), 266);
        assert!(excerpt.ends_with("..."));
    }

    #[test]
    fn tool_only_summary_is_safe_for_multibyte_results() {
        let summary = tool_only_summary(3, Some(&"界".repeat(201)));
        assert!(summary.starts_with("Completed 3 tool call(s). Last result: "));
        // 200 bytes cannot hold 66 whole three-byte scalars plus a partial one.
        assert_eq!(summary.matches('界').count(), 66);
        assert_eq!(
            tool_only_summary(0, None),
            "Completed 0 tool call(s). Last result: ok"
        );
    }
}
