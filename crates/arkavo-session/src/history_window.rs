use crate::conversation::ConversationMessage;

/// Return the trailing slice of `messages` that fits `limit` turns and
/// `token_budget` tokens, cut only on a user turn boundary.
///
/// A tool exchange is indivisible: trimming mid-turn can persist a tool result
/// whose originating call is no longer in the window, which a provider cannot
/// replay. Each user turn is therefore admitted atomically — the whole turn
/// fits in the remaining budget or the window stops before it.
pub fn select_history(
    messages: &[ConversationMessage],
    limit: usize,
    token_budget: usize,
) -> &[ConversationMessage] {
    let mut start = messages.len();
    let mut tokens = 0usize;
    while start > 0 && messages.len() - start < limit {
        let next = messages[..start]
            .iter()
            .rposition(|m| m.role == "user")
            .unwrap_or(0);
        let cost = messages[next..start]
            .iter()
            .fold(0usize, |sum, m| sum.saturating_add(m.token_count));
        if tokens.saturating_add(cost) > token_budget {
            break;
        }
        tokens += cost;
        start = next;
    }
    &messages[start..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn message(role: &str) -> ConversationMessage {
        ConversationMessage {
            id: uuid::Uuid::new_v4(),
            session_id: uuid::Uuid::nil(),
            role: role.into(),
            content: String::new(),
            timestamp: chrono::Utc::now(),
            token_count: 1,
            is_summary: false,
            provider_message: None,
        }
    }

    #[spec("ASTRA-002")]
    #[test]
    fn limits_never_split_a_tool_exchange() {
        let messages = vec![
            message("user"),
            message("assistant"),
            message("tool"),
            message("tool"),
        ];
        assert_eq!(select_history(&messages, 1, 4).len(), 4);
        assert!(select_history(&messages, 1, 3).is_empty());
        assert!(select_history(&messages, 0, 4).is_empty());
    }

    #[spec("ASTRA-002")]
    #[test]
    fn newest_complete_turn_has_priority() {
        let messages = vec![
            message("user"),
            message("assistant"),
            message("user"),
            message("assistant"),
        ];
        assert_eq!(select_history(&messages, 4, 2)[0].id, messages[2].id);
        assert!(select_history(&[], 1, 2).is_empty());
    }
}
