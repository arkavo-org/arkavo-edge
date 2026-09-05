use arkavo_llm::{Message, Role};

/// Return the trailing `limit` turns of `messages`, extended backwards to the
/// user turn that opened them.
///
/// A tool exchange is indivisible: trimming at an arbitrary message can leave
/// an orphan function_call_output that Responses cannot replay. Starting the
/// window on a user message keeps every assistant turn together with the tool
/// results that answer it, so a provider never sees a tool result whose call is
/// missing from the request.
pub fn recent_turns(messages: &[Message], limit: usize) -> Vec<Message> {
    if messages.is_empty() {
        return Vec::new();
    }
    let tentative = messages.len().saturating_sub(limit);
    let start = messages[..=tentative.min(messages.len().saturating_sub(1))]
        .iter()
        .rposition(|message| message.role == Role::User)
        .unwrap_or(0);
    messages[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("ASTRA-002")]
    #[test]
    fn window_keeps_call_and_outputs_together() {
        let mut call = Message::assistant("");
        call.response_items = vec![serde_json::json!({
            "type": "function_call", "call_id": "call_a", "name": "clock",
            "arguments": "{}"
        })];
        let messages = vec![
            Message::user("old turn"),
            Message::assistant("old answer"),
            Message::user("use clock"),
            call,
            Message::tool_result("noon", "call_a", "clock"),
            Message::assistant("It is noon"),
            Message::user("explain"),
        ];
        let window = recent_turns(&messages, 3);
        assert_eq!(window.len(), 5);
        assert_eq!(window[0].content, "use clock");
        assert_eq!(window[1].response_items[0]["call_id"], "call_a");
        assert_eq!(window[2].tool_call_id.as_deref(), Some("call_a"));
    }
}
