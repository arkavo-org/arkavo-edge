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

/// Local templates use tool roles even for text-parsed calls. Responses requires
/// an actual function_call item before a function_call_output can be submitted.
/// The rule is shared with the chat and CLI tool loops, so it lives on the response.
pub(super) fn use_tool_role(response: &arkavo_llm::ProviderResponse) -> bool {
    response.tool_results_use_tool_role()
}

pub(super) fn tool_feedback(
    content: impl Into<String>,
    call_id: impl Into<String>,
    name: impl Into<String>,
    native_role: bool,
) -> Message {
    let content = content.into();
    let name = name.into();
    if native_role {
        Message::tool_result(content, call_id, name)
    } else {
        Message::user(format!("[Tool result {name}]: {content}"))
    }
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
    #[spec("ASTRA-002")]
    #[test]
    fn text_extracted_response_tools_use_user_feedback() {
        let response = arkavo_llm::ProviderResponse {
            provider_state: arkavo_llm::ProviderState::openai_responses(vec![
                json!({"type":"message", "content":[]}),
            ]),
            tool_calls: vec![arkavo_llm::ParsedToolCall {
                tool_name: "read".into(),
                arguments: json!({}),
                call_id: None,
            }],
            ..Default::default()
        };
        let feedback = tool_feedback("result", "synthetic", "read", use_tool_role(&response));
        assert_eq!(feedback.role, Role::User);
        assert!(feedback.tool_call_id.is_none());
        assert!(use_tool_role(&arkavo_llm::ProviderResponse::default()));
        let response = arkavo_llm::ProviderResponse {
            provider_state: batch(&["native"]).provider_state,
            ..Default::default()
        };
        let feedback = tool_feedback("result", "native", "read", use_tool_role(&response));
        assert_eq!(feedback.role, Role::Tool);
        assert_eq!(feedback.tool_call_id.as_deref(), Some("native"));
    }
}
