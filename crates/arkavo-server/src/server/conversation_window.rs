use std::collections::VecDeque;
use std::sync::Arc;

use crate::server::token_estimator::TokenEstimator;

/// Token-budget-aware sliding window of conversation history.
/// Single owner — the event loop task. No Arc<RwLock>.
pub struct ConversationWindow {
    system_message: Option<arkavo_llm::Message>,
    history: VecDeque<arkavo_llm::Message>,
    history_tokens: usize,
    max_history_tokens: usize,
    estimator: Arc<dyn TokenEstimator>,
}

impl ConversationWindow {
    /// `min_feasible_context`: minimum context size across currently-loaded models.
    /// History budget = 70% of that.
    pub fn new(min_feasible_context: usize, estimator: Arc<dyn TokenEstimator>) -> Self {
        Self {
            system_message: None,
            history: VecDeque::new(),
            history_tokens: 0,
            max_history_tokens: min_feasible_context * 70 / 100,
            estimator,
        }
    }

    pub fn set_system_message(&mut self, msg: arkavo_llm::Message) {
        self.system_message = Some(msg);
    }

    /// Append a message and trim oldest if over budget.
    /// Keeps minimum 2 most recent messages (last user + assistant pair).
    pub fn push(&mut self, msg: arkavo_llm::Message) {
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
    pub fn build_messages(&self, system_suffix: Option<&str>) -> Vec<arkavo_llm::Message> {
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

        messages.extend(self.history.iter().cloned());
        messages
    }

    /// Hard reset after degenerate output detection.
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.history_tokens = 0;
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
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
}
