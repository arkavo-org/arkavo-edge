use std::collections::VecDeque;
use std::sync::Arc;

use crate::server::token_estimator::TokenEstimator;

/// Token-budget-aware sliding window of conversation history.
/// Single owner — the event loop task. No Arc<RwLock>.
pub(super) struct ConversationWindow {
    system_message: Option<arkavo_llm::Message>,
    history: VecDeque<arkavo_llm::Message>,
    history_tokens: usize,
    max_history_tokens: usize,
    estimator: Arc<dyn TokenEstimator>,
}

/// Serializable snapshot of a single message in the conversation window.
#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct MessageSnapshot {
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) tokens_est: usize,
}

impl ConversationWindow {
    /// `min_feasible_context`: minimum context size across currently-loaded models.
    /// History budget = 70% of that.
    pub(super) fn new(min_feasible_context: usize, estimator: Arc<dyn TokenEstimator>) -> Self {
        Self {
            system_message: None,
            history: VecDeque::new(),
            history_tokens: 0,
            max_history_tokens: min_feasible_context * 70 / 100,
            estimator,
        }
    }

    pub(super) fn set_system_message(&mut self, msg: arkavo_llm::Message) {
        self.system_message = Some(msg);
    }

    /// Append a message and trim oldest if over budget.
    /// Keeps minimum 2 most recent messages (last user + assistant pair).
    pub(super) fn push(&mut self, msg: arkavo_llm::Message) {
        self.history_tokens += self.estimator.estimate_token_count(&msg.content);
        self.history.push_back(msg);
        while self.history.len() > 2 && self.history_tokens > self.max_history_tokens() {
            if let Some(oldest) = self.history.pop_front() {
                self.history_tokens = self
                    .history_tokens
                    .saturating_sub(self.estimator.estimate_token_count(&oldest.content));
            }
        }
    }

    /// Build the full message list for a conductor call.
    /// Returns [system + optional suffix, history].
    /// Does NOT include the current cycle's user message — caller pushes
    /// that to history before calling this.
    /// Ensures alternating user/assistant roles (required by Gemma/Llama chat templates).
    pub(super) fn build_messages(&self, system_suffix: Option<&str>) -> Vec<arkavo_llm::Message> {
        let mut messages = Vec::with_capacity(self.history.len() + 1);

        if let Some(ref sys) = self.system_message {
            let sys_msg = match system_suffix {
                Some(suffix) => {
                    arkavo_llm::Message::system(format!("{}\n\n{}", sys.content, suffix))
                }
                None => sys.clone(),
            };
            messages.push(sys_msg);
        }

        // Enforce alternating roles: coalesce consecutive same-role messages.
        // This happens when error cycles push a user message without an assistant response.
        let mut last_role: Option<arkavo_llm::Role> = None;
        for msg in &self.history {
            if last_role == Some(msg.role.clone()) {
                // Same role as previous — merge into last message
                if let Some(last) = messages.last_mut() {
                    last.content.push_str("\n\n");
                    last.content.push_str(&msg.content);
                }
            } else {
                last_role = Some(msg.role.clone());
                messages.push(msg.clone());
            }
        }

        messages
    }

    /// Hard reset after degenerate output detection.
    pub(super) fn clear_history(&mut self) {
        self.history.clear();
        self.history_tokens = 0;
    }

    pub(super) fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Return a serializable snapshot of the full conversation (system + history).
    pub(super) fn snapshot_messages(&self) -> Vec<MessageSnapshot> {
        let mut out = Vec::with_capacity(self.history.len() + 1);
        if let Some(ref sys) = self.system_message {
            out.push(MessageSnapshot {
                role: "system".to_string(),
                content: sys.content.clone(),
                tokens_est: self.estimator.estimate_token_count(&sys.content),
            });
        }
        for msg in &self.history {
            out.push(MessageSnapshot {
                role: match msg.role {
                    arkavo_llm::Role::User => "user",
                    arkavo_llm::Role::Assistant => "assistant",
                    arkavo_llm::Role::System => "system",
                    arkavo_llm::Role::Tool => "tool",
                }
                .to_string(),
                content: msg.content.clone(),
                tokens_est: self.estimator.estimate_token_count(&msg.content),
            });
        }
        out
    }

    /// Total token estimate for the current window.
    pub(super) fn total_tokens_est(&self) -> usize {
        let sys_tokens = self
            .system_message
            .as_ref()
            .map(|m| self.estimator.estimate_token_count(&m.content))
            .unwrap_or(0);
        sys_tokens + self.history_tokens
    }

    /// Max history token budget.
    pub(super) fn budget_tokens(&self) -> usize {
        self.max_history_tokens
    }

    fn max_history_tokens(&self) -> usize {
        self.max_history_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::token_estimator::MockEstimator;
    use arkavo_test_macros::spec;

    fn make_window(max_context: usize) -> ConversationWindow {
        ConversationWindow::new(max_context, Arc::new(MockEstimator))
    }

    #[spec("SRV-010")]
    #[test]
    fn test_empty_window_builds_system_only() {
        let mut w = make_window(1000);
        w.set_system_message(arkavo_llm::Message::system("You are an agent."));
        let msgs = w.build_messages(None);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "You are an agent.");
    }

    #[spec("SRV-010")]
    #[test]
    fn test_push_and_build_includes_history() {
        let mut w = make_window(1000);
        w.set_system_message(arkavo_llm::Message::system("system"));
        w.push(arkavo_llm::Message::user("hello"));
        w.push(arkavo_llm::Message::assistant("world"));
        let msgs = w.build_messages(None);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1].content, "hello");
        assert_eq!(msgs[2].content, "world");
    }

    #[spec("SRV-010")]
    #[test]
    fn test_system_suffix_appended_without_mutation() {
        let mut w = make_window(1000);
        w.set_system_message(arkavo_llm::Message::system("base"));
        let msgs = w.build_messages(Some("## Control Signals\nDuplicate detected"));
        assert!(msgs[0].content.contains("base"));
        assert!(msgs[0].content.contains("Control Signals"));
        // Original system message unchanged
        let msgs2 = w.build_messages(None);
        assert_eq!(msgs2[0].content, "base");
    }

    #[spec("SRV-010")]
    #[test]
    fn test_trimming_removes_oldest() {
        // MockEstimator: 4 chars = 1 token. Max context = 100 -> budget = 70 tokens.
        // Each message is 280 chars = 70 tokens.
        let mut w = make_window(100);
        let long_msg = "x".repeat(280);
        w.push(arkavo_llm::Message::user(&long_msg));
        assert_eq!(w.history_len(), 1);
        w.push(arkavo_llm::Message::assistant(&long_msg));
        assert_eq!(w.history_len(), 2);
        w.push(arkavo_llm::Message::user(&long_msg));
        assert_eq!(w.history_len(), 2);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_minimum_two_messages_kept() {
        let mut w = make_window(10);
        w.push(arkavo_llm::Message::user("a".repeat(100)));
        w.push(arkavo_llm::Message::assistant("b".repeat(100)));
        assert_eq!(w.history_len(), 2);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_clear_history_resets() {
        let mut w = make_window(1000);
        w.push(arkavo_llm::Message::user("msg1"));
        w.push(arkavo_llm::Message::assistant("msg2"));
        assert_eq!(w.history_len(), 2);
        w.clear_history();
        assert_eq!(w.history_len(), 0);
        w.set_system_message(arkavo_llm::Message::system("sys"));
        w.clear_history();
        let msgs = w.build_messages(None);
        assert_eq!(msgs.len(), 1);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_running_token_total_accuracy() {
        let mut w = make_window(10000);
        w.push(arkavo_llm::Message::user("hello"));
        assert_eq!(w.history_tokens, 1);
        w.push(arkavo_llm::Message::assistant("world!!"));
        assert_eq!(w.history_tokens, 2);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_build_without_system_message() {
        let mut w = make_window(1000);
        w.push(arkavo_llm::Message::user("hello"));
        let msgs = w.build_messages(None);
        assert_eq!(msgs.len(), 1);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_consecutive_user_messages_coalesced() {
        let mut w = make_window(10000);
        w.set_system_message(arkavo_llm::Message::system("sys"));
        // Simulate two error cycles: user pushed, no assistant response
        w.push(arkavo_llm::Message::user("cycle 1 prompt"));
        w.push(arkavo_llm::Message::user("cycle 2 prompt"));
        w.push(arkavo_llm::Message::assistant("response"));

        let msgs = w.build_messages(None);
        // system + coalesced_user + assistant = 3 (not 4)
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, arkavo_llm::Role::System);
        assert_eq!(msgs[1].role, arkavo_llm::Role::User);
        assert!(msgs[1].content.contains("cycle 1 prompt"));
        assert!(msgs[1].content.contains("cycle 2 prompt"));
        assert_eq!(msgs[2].role, arkavo_llm::Role::Assistant);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_alternating_roles_preserved() {
        let mut w = make_window(10000);
        w.push(arkavo_llm::Message::user("u1"));
        w.push(arkavo_llm::Message::assistant("a1"));
        w.push(arkavo_llm::Message::user("u2"));
        w.push(arkavo_llm::Message::assistant("a2"));

        let msgs = w.build_messages(None);
        // Already alternating — no coalescing needed
        assert_eq!(msgs.len(), 4);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_snapshot_messages_returns_all() {
        let mut w = make_window(1000);
        w.set_system_message(arkavo_llm::Message::system("You are an agent."));
        w.push(arkavo_llm::Message::user("hello"));
        w.push(arkavo_llm::Message::assistant("world"));

        let snapshot = w.snapshot_messages();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].role, "system");
        assert_eq!(snapshot[0].content, "You are an agent.");
        assert_eq!(snapshot[1].role, "user");
        assert_eq!(snapshot[2].role, "assistant");
    }

    #[spec("SRV-010")]
    #[test]
    fn test_snapshot_messages_empty_window() {
        let w = make_window(1000);
        let snapshot = w.snapshot_messages();
        assert!(snapshot.is_empty());
    }

    #[spec("SRV-010")]
    #[test]
    fn test_snapshot_messages_includes_token_estimates() {
        let mut w = make_window(1000);
        w.push(arkavo_llm::Message::user("hello world test msg"));
        let snapshot = w.snapshot_messages();
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot[0].tokens_est > 0);
    }
}
