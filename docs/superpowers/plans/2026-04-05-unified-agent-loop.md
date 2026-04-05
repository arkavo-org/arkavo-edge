# Unified Agent Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the inline orchestrator loop in `a2a_server.rs` with a `tokio::select!` event loop that unifies tick, A2A message, and human override processing through a single queue with persistent conversation context.

**Architecture:** New modules `agent_event.rs`, `token_estimator.rs`, `conversation_window.rs`, and `agent_loop.rs` in `crates/arkavo-server/src/server/`. The event loop owns a `ConversationWindow` (persistent history) and processes `AgentEvent`s. `ToolMemory` transitions from history replay to control-signal-only injection. The conductor gains an `existing_messages` parameter for pre-built conversation history.

**Tech Stack:** Rust, tokio (select!, mpsc, oneshot, time), llama.cpp FFI (tokenization), arkavo-llm Message types.

**Spec:** `docs/superpowers/specs/2026-04-05-unified-agent-loop-design.md`

---

## Phase 1: Standalone Modules (zero integration risk)

### Task 1: Create agent_event.rs types

**Files:**
- Create: `crates/arkavo-server/src/server/agent_event.rs`

- [ ] **Step 1: Write the types file**

```rust
// crates/arkavo-server/src/server/agent_event.rs
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CycleId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CorrelationId(pub uuid::Uuid);

#[derive(Debug)]
pub enum MessageDisposition {
    /// Included in the current cycle's prompt
    Incorporated { cycle_id: CycleId },
    /// Queued for next cycle (current cycle was already assembling)
    Deferred,
    /// Rejected (budget exceeded, agent shutting down, etc.)
    Rejected { reason: String },
}

#[derive(Debug)]
pub struct CycleReceipt {
    pub cycle_id: CycleId,
    pub correlation_id: CorrelationId,
    pub disposition: MessageDisposition,
}

pub enum AgentEvent {
    IncomingMessage {
        sender: String,
        content: String,
        task_id: uuid::Uuid,
        correlation_id: CorrelationId,
        reply: oneshot::Sender<CycleReceipt>,
    },
    HumanOverride {
        instruction: String,
        correlation_id: CorrelationId,
        reply: oneshot::Sender<CycleReceipt>,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePriority {
    Normal,
    Override,
}

pub struct PendingMessage {
    pub content: String,
    pub task_id: Option<uuid::Uuid>,
    pub correlation_id: CorrelationId,
    pub reply: Option<oneshot::Sender<CycleReceipt>>,
    pub priority: MessagePriority,
}
```

- [ ] **Step 2: Add module declaration to mod.rs**

Add to `crates/arkavo-server/src/server/mod.rs` after the existing module declarations (around line 31):

```rust
mod agent_event;
```

Add re-exports after the existing pub use block (around line 56):

```rust
pub use agent_event::{
    AgentEvent, AgentLoopConfig, CorrelationId, CycleId, CycleReceipt, MessageDisposition,
    MessagePriority, PendingMessage,
};
```

Note: `AgentLoopConfig` will be added in Task 5 when `agent_loop.rs` is created. For now, exclude it from the re-export and add it later.

- [ ] **Step 3: Verify compilation**

Run: `cargo build -p arkavo-server -q`
Expected: Compiles with no errors.

- [ ] **Step 4: Write unit tests**

Add `#[cfg(test)]` module at the bottom of `agent_event.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SRV-010")]
    #[test]
    fn test_cycle_id_is_copy() {
        let id = CycleId(42);
        let copy = id;
        assert_eq!(id, copy);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_correlation_id_is_copy() {
        let id = CorrelationId(uuid::Uuid::new_v4());
        let copy = id;
        assert_eq!(id, copy);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_pending_message_priority_ordering() {
        let mut messages: Vec<PendingMessage> = Vec::new();

        let normal = PendingMessage {
            content: "normal".into(),
            task_id: None,
            correlation_id: CorrelationId(uuid::Uuid::new_v4()),
            reply: None,
            priority: MessagePriority::Normal,
        };
        messages.push(normal);

        let override_msg = PendingMessage {
            content: "override".into(),
            task_id: None,
            correlation_id: CorrelationId(uuid::Uuid::new_v4()),
            reply: None,
            priority: MessagePriority::Override,
        };
        // Override inserts at front
        messages.insert(0, override_msg);

        assert_eq!(messages[0].priority, MessagePriority::Override);
        assert_eq!(messages[1].priority, MessagePriority::Normal);
    }

    #[spec("SRV-010")]
    #[tokio::test]
    async fn test_cycle_receipt_flows_through_oneshot() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let correlation_id = CorrelationId(uuid::Uuid::new_v4());

        let receipt = CycleReceipt {
            cycle_id: CycleId(5),
            correlation_id,
            disposition: MessageDisposition::Incorporated {
                cycle_id: CycleId(5),
            },
        };
        tx.send(receipt).unwrap();

        let received = rx.await.unwrap();
        assert_eq!(received.cycle_id, CycleId(5));
        assert_eq!(received.correlation_id, correlation_id);
        assert!(matches!(
            received.disposition,
            MessageDisposition::Incorporated { .. }
        ));
    }

    #[spec("SRV-010")]
    #[tokio::test]
    async fn test_dropped_sender_returns_error() {
        let (tx, rx) = tokio::sync::oneshot::channel::<CycleReceipt>();
        drop(tx);
        assert!(rx.await.is_err());
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p arkavo-server agent_event -- --nocapture`
Expected: All 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-server/src/server/agent_event.rs crates/arkavo-server/src/server/mod.rs
git commit -m "Agent event types for unified loop — CycleReceipt, CorrelationId, PendingMessage"
```

---

### Task 2: Create token_estimator.rs

**Files:**
- Create: `crates/arkavo-server/src/server/token_estimator.rs`
- Modify: `crates/arkavo-server/src/server/mod.rs` (add module)

The primary `LlamaTokenEstimator` wraps the already-loaded model's tokenizer via llama.cpp FFI. No vendored merge table — the tokenizer is already in memory.

- [ ] **Step 1: Write the trait and implementations**

```rust
// crates/arkavo-server/src/server/token_estimator.rs

/// Budget-gate token estimator for ConversationWindow trimming.
/// Not a billing meter — trimming decisions tolerate 5-10% error
/// because the 70% budget has 30% headroom.
pub trait TokenEstimator: Send + Sync {
    fn estimate_tokens(&self, text: &str) -> usize;

    /// Counting-only fast path. Implementors can skip allocating
    /// a full token vector. Default: delegates to estimate_tokens.
    fn estimate_token_count(&self, text: &str) -> usize {
        self.estimate_tokens(text)
    }
}

/// Wraps an already-loaded llama.cpp model's tokenizer.
/// llama_tokenize() is a pure vocabulary lookup — no GPU, no inference.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct LlamaTokenEstimator {
    model: std::sync::Arc<arkavo_llama_cpp::LlamaModel>,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl LlamaTokenEstimator {
    pub fn new(model: std::sync::Arc<arkavo_llama_cpp::LlamaModel>) -> Self {
        Self { model }
    }
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl TokenEstimator for LlamaTokenEstimator {
    fn estimate_tokens(&self, text: &str) -> usize {
        self.estimate_token_count(text)
    }

    fn estimate_token_count(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        let vocab = self.model.get_vocab();
        match arkavo_llama_cpp::tokenize_with_model(vocab, text.as_bytes()) {
            Ok(tokens) => tokens.len(),
            Err(_) => text.len() / 4, // fallback on tokenization error
        }
    }
}

/// Deterministic estimator for tests. Uses chars/4.
pub struct MockEstimator;

impl TokenEstimator for MockEstimator {
    fn estimate_tokens(&self, text: &str) -> usize {
        self.estimate_token_count(text)
    }

    fn estimate_token_count(&self, text: &str) -> usize {
        // Integer division, minimum 1 for non-empty text
        if text.is_empty() { 0 } else { (text.len() / 4).max(1) }
    }
}
```

- [ ] **Step 2: Add module declaration to mod.rs**

Add to `crates/arkavo-server/src/server/mod.rs`:

```rust
mod token_estimator;
```

Add re-export:

```rust
pub use token_estimator::TokenEstimator;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build -p arkavo-server -q`
Expected: Compiles. Note: `LlamaTokenEstimator` is feature-gated — build both with and without `llama-cpp` if possible.

- [ ] **Step 4: Write unit tests**

Add `#[cfg(test)]` module at the bottom of `token_estimator.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SRV-010")]
    #[test]
    fn test_mock_estimator_empty_string() {
        let estimator = MockEstimator;
        assert_eq!(estimator.estimate_token_count(""), 0);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_mock_estimator_short_string() {
        let estimator = MockEstimator;
        // "hi" is 2 chars, 2/4 = 0, but min 1 for non-empty
        assert_eq!(estimator.estimate_token_count("hi"), 1);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_mock_estimator_longer_string() {
        let estimator = MockEstimator;
        // 100 chars / 4 = 25
        let text = "a".repeat(100);
        assert_eq!(estimator.estimate_token_count(&text), 25);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_mock_estimator_is_deterministic() {
        let estimator = MockEstimator;
        let text = "Hello, world! This is a test of token estimation.";
        let count1 = estimator.estimate_token_count(text);
        let count2 = estimator.estimate_token_count(text);
        assert_eq!(count1, count2);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_default_estimate_token_count_delegates() {
        let estimator = MockEstimator;
        let text = "test string for delegation";
        assert_eq!(
            estimator.estimate_tokens(text),
            estimator.estimate_token_count(text)
        );
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p arkavo-server token_estimator -- --nocapture`
Expected: All 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-server/src/server/token_estimator.rs crates/arkavo-server/src/server/mod.rs
git commit -m "TokenEstimator trait with LlamaTokenEstimator and MockEstimator"
```

---

### Task 3: Create conversation_window.rs

**Files:**
- Create: `crates/arkavo-server/src/server/conversation_window.rs`
- Modify: `crates/arkavo-server/src/server/mod.rs` (add module)

- [ ] **Step 1: Write the ConversationWindow struct and methods**

```rust
// crates/arkavo-server/src/server/conversation_window.rs
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
                self.history_tokens =
                    self.history_tokens
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
```

- [ ] **Step 2: Add module declaration to mod.rs**

Add to `crates/arkavo-server/src/server/mod.rs`:

```rust
mod conversation_window;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build -p arkavo-server -q`
Expected: Compiles.

- [ ] **Step 4: Write unit tests**

Add `#[cfg(test)]` module at the bottom of `conversation_window.rs`:

```rust
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
        assert_eq!(msgs.len(), 3); // system + 2 history
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

        // Second message pushes over budget (140 tokens > 70 budget)
        w.push(arkavo_llm::Message::assistant(&long_msg));
        // Both kept because minimum is 2
        assert_eq!(w.history_len(), 2);

        // Third message: now 3 msgs, 210 tokens > 70. Trim oldest.
        w.push(arkavo_llm::Message::user(&long_msg));
        // Should trim down to 2 (the last two messages)
        assert_eq!(w.history_len(), 2);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_minimum_two_messages_kept() {
        // Even with very small budget, keep at least 2 messages
        let mut w = make_window(10); // budget = 7 tokens
        w.push(arkavo_llm::Message::user("a".repeat(100))); // 25 tokens
        w.push(arkavo_llm::Message::assistant("b".repeat(100))); // 25 tokens
        // Both kept despite being way over budget
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

        // System message preserved after clear
        w.set_system_message(arkavo_llm::Message::system("sys"));
        w.clear_history();
        let msgs = w.build_messages(None);
        assert_eq!(msgs.len(), 1); // system only
    }

    #[spec("SRV-010")]
    #[test]
    fn test_running_token_total_accuracy() {
        let mut w = make_window(10000);
        // MockEstimator: "hello" (5 chars) = 1 token, "world!!" (7 chars) = 1 token
        w.push(arkavo_llm::Message::user("hello"));
        assert_eq!(w.history_tokens, 1);

        w.push(arkavo_llm::Message::assistant("world!!"));
        assert_eq!(w.history_tokens, 2);
    }

    #[spec("SRV-010")]
    #[test]
    fn test_build_without_system_message() {
        let mut w = make_window(1000);
        // No system message set
        w.push(arkavo_llm::Message::user("hello"));
        let msgs = w.build_messages(None);
        assert_eq!(msgs.len(), 1); // just history, no system
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p arkavo-server conversation_window -- --nocapture`
Expected: All 8 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-server/src/server/conversation_window.rs crates/arkavo-server/src/server/mod.rs
git commit -m "ConversationWindow — VecDeque history with O(1) token-budget trimming"
```

---

### Task 4: Add Router::min_feasible_context_size()

**Files:**
- Modify: `crates/arkavo-router/src/lib.rs` (add method to Router, around line 833)

- [ ] **Step 1: Write the failing test**

Add to the test module in `crates/arkavo-router/src/lib.rs` (or the appropriate test file):

```rust
#[spec("RTR-010")]
#[tokio::test]
async fn test_min_feasible_context_size_default() {
    let router = Router::new_offline().await.unwrap();
    // No models loaded — should return conservative default
    let size = router.min_feasible_context_size();
    assert_eq!(size, 4096);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p arkavo-router min_feasible_context_size`
Expected: FAIL — method does not exist.

- [ ] **Step 3: Implement the method**

Add to `Router` impl block in `crates/arkavo-router/src/lib.rs` (after `fastest_local_model()` around line 833):

```rust
/// Minimum context size across all currently-loaded local models.
/// Returns conservative default (4096) if no models are loaded.
/// Used by ConversationWindow to compute the history token budget.
#[cfg(feature = "llama-cpp")]
pub fn min_feasible_context_size(&self) -> usize {
    let models = self.model_registry.loaded_model_names();
    if models.is_empty() {
        return 4096;
    }
    let mut min_ctx = usize::MAX;
    for name in &models {
        if let Some(model) = self.model_registry.get(name) {
            let ctx = model.get_trained_context_size() as usize;
            if ctx < min_ctx {
                min_ctx = ctx;
            }
        }
    }
    if min_ctx == usize::MAX { 4096 } else { min_ctx }
}

#[cfg(not(feature = "llama-cpp"))]
pub fn min_feasible_context_size(&self) -> usize {
    4096
}
```

Note: Check if `ModelRegistry` has a `loaded_model_names()` method. If not, add one that returns `Vec<String>` from the keys of the models HashMap. Check the existing API:

```rust
// In model_registry.rs, if loaded_model_names() doesn't exist, add:
pub fn loaded_model_names(&self) -> Vec<String> {
    self.models.read().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p arkavo-router min_feasible_context_size`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/arkavo-router/src/lib.rs
# If model_registry.rs was modified:
git add crates/arkavo-llm/src/model_registry.rs
git commit -m "Router::min_feasible_context_size() for ConversationWindow budget"
```

---

### Task 5: Add Router::token_estimator()

**Files:**
- Modify: `crates/arkavo-router/src/lib.rs` (add method)

This provides a way for `AgentLoopConfig` construction to get a `TokenEstimator` from the router's loaded model.

- [ ] **Step 1: Write the method**

Add to `Router` impl block, near `min_feasible_context_size()`:

```rust
/// Get an Arc<LlamaModel> from any loaded model for token estimation.
/// Returns None if no models are loaded.
#[cfg(feature = "llama-cpp")]
pub fn any_loaded_model(&self) -> Option<std::sync::Arc<arkavo_llama_cpp::LlamaModel>> {
    let names = self.model_registry.loaded_model_names();
    names.first().and_then(|name| self.model_registry.get(name))
}
```

This is used at agent startup to construct `LlamaTokenEstimator`. The caller handles the `None` case by falling back to `MockEstimator` (or panicking if no model is loaded, which shouldn't happen for orchestrators).

- [ ] **Step 2: Verify compilation**

Run: `cargo build -p arkavo-router -q`
Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-router/src/lib.rs
git commit -m "Router::any_loaded_model() for TokenEstimator construction"
```

---

## Phase 2: Event Loop Integration (high-risk, overnight test gate)

### Task 6: Add existing_messages parameter to conductor.rs

**Files:**
- Modify: `crates/arkavo-server/src/server/conductor.rs:65-80` (signature), `393-412` (message construction)

- [ ] **Step 1: Add the parameter to the function signature**

In `crates/arkavo-server/src/server/conductor.rs`, modify `execute_with_conductor_and_learning` signature (line 65-80). Add `existing_messages: Option<Vec<arkavo_llm::Message>>` as the last parameter before the return type (before the `#[cfg(feature = "iroh")]` line, or after `compute_budget`):

```rust
pub async fn execute_with_conductor_and_learning(
    conductor: &Arc<Conductor<InMemoryTaskStore>>,
    router: &Arc<arkavo_router::Router>,
    mcp_registry: &Arc<McpRegistry>,
    task_content: String,
    task_id: Option<uuid::Uuid>,
    task_executor: Option<&Arc<TaskExecutor>>,
    learning_bus: Option<&Arc<LearningBus>>,
    tool_memory: Option<&Arc<tokio::sync::RwLock<ToolMemory>>>,
    system_prompt: Option<&str>,
    mesh_state: Option<&Arc<arkavo_mcp_mesh::MeshToolsState>>,
    model_hint: Option<&arkavo_router::ModelChoice>,
    images: Option<Vec<String>>,
    compute_budget: Option<&arkavo_budget::SharedComputeBudget>,
    existing_messages: Option<Vec<arkavo_llm::Message>>,
    #[cfg(feature = "iroh")] iroh_node: Option<&Arc<arkavo_tdf_iroh::IrohNode>>,
) -> std::result::Result<String, String>
```

- [ ] **Step 2: Modify the message construction block**

At lines 393-412 in `conductor.rs`, replace the message construction with:

```rust
let messages = if let Some(existing) = existing_messages {
    existing
} else {
    let mut messages = Vec::new();
    let merged_system = match (system_prompt, &rlm_system_prompt) {
        (Some(sys), Some(rlm)) => Some(format!("{sys}\n\n{rlm}")),
        (Some(sys), None) => Some(sys.to_string()),
        (None, Some(rlm)) => Some(rlm.clone()),
        (None, None) => None,
    };
    if let Some(sys) = merged_system {
        messages.push(arkavo_llm::Message::system(sys));
    }
    if let Some(imgs) = images {
        messages.push(arkavo_llm::Message::user_with_images(augmented_content, imgs));
    } else {
        messages.push(arkavo_llm::Message::user(augmented_content));
    }
    messages
};
```

- [ ] **Step 3: Update all existing callers to pass None**

Search for all call sites of `execute_with_conductor_and_learning`:

```bash
grep -rn "execute_with_conductor_and_learning" crates/arkavo-server/src/
```

For each call site, add `None,` (for `existing_messages`) before the `#[cfg(feature = "iroh")]` argument. Key locations:
- `a2a_server.rs` orchestrator loop call (~line 1554)
- `handlers/messaging.rs` spawned call (~line 272)
- Any other callers found by grep

- [ ] **Step 4: Update the `execute_with_conductor` wrapper**

The simpler `execute_with_conductor` wrapper in `conductor.rs` (or `mod.rs`) likely calls `execute_with_conductor_and_learning`. Add `None` for `existing_messages` there too.

- [ ] **Step 5: Verify compilation**

Run: `cargo build -p arkavo-server -q`
Expected: Compiles with no errors. All callers pass `None` — zero behavioral change.

- [ ] **Step 6: Run existing tests**

Run: `cargo test -p arkavo-server`
Expected: All existing tests pass — this is a backward-compatible change.

- [ ] **Step 7: Commit**

```bash
git add crates/arkavo-server/src/server/conductor.rs crates/arkavo-server/src/server/a2a_server.rs crates/arkavo-server/src/server/handlers/messaging.rs
# Add any other files with updated call sites
git commit -m "Conductor: add existing_messages parameter for pre-built conversation history"
```

---

### Task 7: Extract helper functions into agent_loop.rs

**Files:**
- Create: `crates/arkavo-server/src/server/agent_loop.rs`
- Modify: `crates/arkavo-server/src/server/a2a_server.rs` (remove helpers)
- Modify: `crates/arkavo-server/src/server/mod.rs` (add module)

Move these pure functions from `a2a_server.rs` to `agent_loop.rs`:
- `detect_urgency()` (a2a_server.rs:1819-1846)
- `compact_observation()` (a2a_server.rs:1853-1874)
- `compute_per_agent_bytes_static()` (a2a_server.rs:1882-1895)
- `broadcast_state_to_peers()` (a2a_server.rs:1974-2061)
- `send_advisory_task()` (a2a_server.rs:2067+)
- `stage_on_iroh()` (a2a_server.rs:1951-1972, feature-gated)

- [ ] **Step 1: Create agent_loop.rs with the extracted helpers**

Copy each function from `a2a_server.rs` into `crates/arkavo-server/src/server/agent_loop.rs`. Change visibility from private to `pub(super)` so they're accessible within the server module. Keep function signatures identical — these are pure functions of their arguments.

At the top, add necessary imports:

```rust
// crates/arkavo-server/src/server/agent_loop.rs
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::server::agent_event::*;
use crate::server::conversation_window::ConversationWindow;
use crate::server::token_estimator::TokenEstimator;
use crate::server::tool_memory::ToolMemory;
```

Also add the `AgentLoopConfig` struct here (this is where it lives):

```rust
pub struct AgentLoopConfig {
    pub conductor: Arc<super::conductor::Conductor<super::InMemoryTaskStore>>,
    pub router: Arc<arkavo_router::Router>,
    pub mcp_registry: Arc<arkavo_mcp_tools::McpRegistry>,
    pub agent_memory: Arc<RwLock<ToolMemory>>,
    pub learning_bus: Option<Arc<super::learning_bus::LearningBus>>,
    pub mesh_state: Arc<arkavo_mcp_mesh::MeshToolsState>,
    pub compute_budget: arkavo_budget::SharedComputeBudget,
    pub model_hint: Option<arkavo_router::ModelChoice>,
    pub purpose: String,
    pub orchestrator_tick: Arc<std::sync::atomic::AtomicU64>,
    pub has_mcp_tools: bool,
    pub tool_loop_budget: Option<u32>,
    #[cfg(feature = "iroh")]
    pub iroh_node: Option<Arc<arkavo_tdf_iroh::IrohNode>>,
}
```

- [ ] **Step 2: Remove the helpers from a2a_server.rs**

Delete the function bodies from `a2a_server.rs` for each moved function. If any are called from within `a2a_server.rs` during the transition (the main loop still references them), temporarily re-export them:

```rust
// In a2a_server.rs, temporarily:
use super::agent_loop::{detect_urgency, compact_observation, ...};
```

- [ ] **Step 3: Add module declaration to mod.rs**

```rust
mod agent_loop;
```

Add `AgentLoopConfig` to the re-exports:

```rust
pub use agent_event::{
    AgentEvent, AgentLoopConfig, CorrelationId, CycleId, CycleReceipt, MessageDisposition,
    MessagePriority, PendingMessage,
};
```

Wait — `AgentLoopConfig` lives in `agent_loop.rs`, not `agent_event.rs`. Fix the re-export:

```rust
pub use agent_loop::AgentLoopConfig;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build -p arkavo-server -q`
Expected: Compiles. The main loop still works because the helpers are re-imported.

- [ ] **Step 5: Run existing tests**

Run: `cargo test -p arkavo-server`
Expected: All existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-server/src/server/agent_loop.rs crates/arkavo-server/src/server/a2a_server.rs crates/arkavo-server/src/server/mod.rs
git commit -m "Extract orchestrator helper functions to agent_loop.rs"
```

---

### Task 8: Implement the tokio::select! event loop

**Files:**
- Modify: `crates/arkavo-server/src/server/agent_loop.rs` (add `run_agent_loop`)

This is the core task — the `run_agent_loop` function that replaces the inline loop.

- [ ] **Step 1: Add new helper functions for the event loop**

Add to `agent_loop.rs`:

```rust
/// Drain pending messages into a prompt block and produce CycleReceipts.
fn drain_pending_messages(
    pending: &mut Vec<PendingMessage>,
    cycle_id: CycleId,
) -> (String, Vec<(oneshot::Sender<CycleReceipt>, CycleReceipt)>) {
    let mut block = String::new();
    let mut receipts = Vec::new();

    for mut msg in pending.drain(..) {
        if !block.is_empty() {
            block.push('\n');
        }
        block.push_str(&msg.content);

        if let Some(reply) = msg.reply.take() {
            receipts.push((
                reply,
                CycleReceipt {
                    cycle_id,
                    correlation_id: msg.correlation_id,
                    disposition: MessageDisposition::Incorporated { cycle_id },
                },
            ));
        }
    }

    (block, receipts)
}
```

- [ ] **Step 2: Implement run_agent_loop**

Add to `agent_loop.rs`. This function replicates the behavior of `a2a_server.rs:1352-1728` but uses `tokio::select!` and `ConversationWindow`:

```rust
pub async fn run_agent_loop(
    config: AgentLoopConfig,
    mut agent_event_rx: tokio::sync::mpsc::Receiver<AgentEvent>,
) {
    // --- Persistent state (from a2a_server.rs:1352-1357) ---
    let mut cycle: u64 = 0;
    let mut consecutive_no_action_cycles: u32 = 0;
    let mut last_broadcast_cycle: u64 = 0;
    let mut consecutive_timeouts: u32 = 0;
    let mut last_cycle_prompt_hash: u64 = 0;
    let mut consecutive_duplicate_prompts: u32 = 0;

    // --- New state ---
    let estimator: Arc<dyn TokenEstimator> = {
        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        {
            match config.router.any_loaded_model() {
                Some(model) => Arc::new(
                    crate::server::token_estimator::LlamaTokenEstimator::new(model),
                ),
                None => Arc::new(crate::server::token_estimator::MockEstimator),
            }
        }
        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            Arc::new(crate::server::token_estimator::MockEstimator)
        }
    };
    let min_context = config.router.min_feasible_context_size();
    let mut conversation = ConversationWindow::new(min_context, estimator);
    conversation.set_system_message(arkavo_llm::Message::system(&config.purpose));

    // Staging buffer: drained from channel in recv branch, consumed in tick branch.
    // The channel itself is the arrival buffer for messages during execution.
    let mut pending_messages: Vec<PendingMessage> = Vec::new();

    let mut cycle_interval_secs: u64 = 5;
    let mut tick_interval = tokio::time::interval(
        std::time::Duration::from_secs(cycle_interval_secs),
    );
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                cycle += 1;
                config.orchestrator_tick.store(cycle, std::sync::atomic::Ordering::Relaxed);

                // 1. Budget gate (specialists only — commanders never gated)
                if !config.has_mcp_tools {
                    let snapshot = config.compute_budget.read().await.snapshot();
                    if !snapshot.has_remaining {
                        continue;
                    }
                }

                // 2. Drain gossip completions → specialist_context
                // (Migrated from a2a_server.rs:1404-1466)
                // ... [existing gossip drain logic using config.learning_bus and config.mesh_state]

                // 3. Drain pending messages → annotations + send receipts
                let (message_block, receipts) = drain_pending_messages(
                    &mut pending_messages,
                    CycleId(cycle),
                );
                for (reply, receipt) in receipts {
                    let _ = reply.send(receipt);
                }

                // 4. Build control signals from ToolMemory
                // Phase 2: keep format_for_prompt() alongside ConversationWindow
                let control_signals = {
                    let mem = config.agent_memory.read().await;
                    mem.format_for_prompt()
                };

                // 5. Assemble cycle prompt
                // (Migrated from a2a_server.rs:1468-1504)
                // Includes: recent_actions, variety_warning, error_corrections,
                //           specialist_context, dead_man_warning, message_block

                // 6. Duplicate prompt detection
                // (Migrated from a2a_server.rs:1506-1542)

                // 7. Push user message to conversation (unconditionally)
                let user_msg = arkavo_llm::Message::user(&cycle_prompt);
                conversation.push(user_msg);

                // 8. Build messages via ConversationWindow
                let messages = conversation.build_messages(
                    if control_signals.is_empty() { None } else { Some(&control_signals) },
                );

                // 9. Execute via conductor with pre-built messages
                let start = std::time::Instant::now();
                let result = super::conductor::execute_with_conductor_and_learning(
                    &config.conductor,
                    &config.router,
                    &config.mcp_registry,
                    cycle_prompt.clone(),
                    None, // no task_id (orchestrator loop)
                    None, // no task_executor
                    config.learning_bus.as_ref(),
                    Some(&config.agent_memory),
                    None, // system prompt is in messages already
                    Some(&config.mesh_state),
                    config.model_hint.as_ref(),
                    None, // no images
                    config.tool_loop_budget
                        .map(|_| &config.compute_budget)
                        .or(None),
                    Some(messages), // <-- pre-built conversation history
                    #[cfg(feature = "iroh")]
                    config.iroh_node.as_ref(),
                ).await;

                // 10. Push assistant response (on success only)
                match &result {
                    Ok(response_text) => {
                        conversation.push(
                            arkavo_llm::Message::assistant(response_text),
                        );
                        // 11. Update timeout/action tracking
                        // (Migrated from a2a_server.rs:1656-1701)
                        let elapsed = start.elapsed();
                        // ... [existing metric update logic]
                    }
                    Err(e) => {
                        consecutive_timeouts += 1;
                        warn!("Cycle {cycle} error: {e}");
                    }
                }

                // 12. Degenerate reset (8+ no-action cycles)
                if consecutive_no_action_cycles >= 8 {
                    warn!(
                        "Context reset: {} ticks without action — clearing history",
                        consecutive_no_action_cycles
                    );
                    conversation.clear_history();
                    let mut mem = config.agent_memory.write().await;
                    mem.clear();
                    drop(mem);
                    consecutive_no_action_cycles = 0;
                    consecutive_duplicate_prompts = 0;
                    // Fast-path lesson via learning bus
                    if let Some(ref bus) = config.learning_bus {
                        bus.add_fast_lesson(
                            "degenerate_reset",
                            "Agent stuck for 8+ cycles, context reset triggered",
                        ).await;
                    }
                }

                // 13. State broadcast every 3 cycles
                if cycle - last_broadcast_cycle >= 3 {
                    // ... [existing broadcast logic using moved helpers]
                    last_broadcast_cycle = cycle;
                }

                // 14. Update adaptive interval (only when changed)
                let new_interval = match consecutive_timeouts {
                    0 => 5, 1 => 15, 2 => 30, _ => 60,
                };
                if new_interval != cycle_interval_secs {
                    cycle_interval_secs = new_interval;
                    tick_interval = tokio::time::interval(
                        std::time::Duration::from_secs(cycle_interval_secs),
                    );
                    tick_interval.set_missed_tick_behavior(
                        tokio::time::MissedTickBehavior::Delay,
                    );
                }
            }

            Some(event) = agent_event_rx.recv() => {
                match event {
                    AgentEvent::IncomingMessage {
                        sender, content, task_id,
                        correlation_id, reply,
                    } => {
                        pending_messages.push(PendingMessage {
                            content: format!(
                                "[msg correlation_id={} from={}] {}",
                                correlation_id.0, sender, content,
                            ),
                            task_id: Some(task_id),
                            correlation_id,
                            reply: Some(reply),
                            priority: MessagePriority::Normal,
                        });
                        tick_interval.reset();
                    }
                    AgentEvent::HumanOverride {
                        instruction, correlation_id, reply,
                    } => {
                        pending_messages.insert(0, PendingMessage {
                            content: format!(
                                "[msg correlation_id={} from=human] {}",
                                correlation_id.0, instruction,
                            ),
                            task_id: None,
                            correlation_id,
                            reply: Some(reply),
                            priority: MessagePriority::Override,
                        });
                        tick_interval.reset();
                    }
                    AgentEvent::Shutdown => break,
                }
            }
        }
    }

    info!("Agent loop exited");
}
```

**Important implementation note:** The `// ...` comments in the tick branch represent code that must be migrated line-by-line from `a2a_server.rs`. The exact logic for gossip draining (lines 1404-1466), cycle prompt assembly (1468-1504), duplicate detection (1506-1542), metric updates (1656-1701), and state broadcast (1593-1649) must be copied faithfully, using fields from `config` instead of closure captures. Do NOT rewrite these sections — copy and adapt.

- [ ] **Step 3: Verify compilation**

Run: `cargo build -p arkavo-server -q`
Expected: Compiles. The event loop is defined but not yet called — the old loop in `a2a_server.rs` still runs.

- [ ] **Step 4: Commit**

```bash
git add crates/arkavo-server/src/server/agent_loop.rs
git commit -m "Implement tokio::select! event loop with ConversationWindow persistence"
```

---

### Task 9: Replace a2a_server.rs loop with agent_loop

**Files:**
- Modify: `crates/arkavo-server/src/server/a2a_server.rs`

- [ ] **Step 1: Modify start_orchestrator_loop**

Replace the inline loop body in `start_orchestrator_loop()` (lines ~1339-1728) with:

```rust
pub async fn start_orchestrator_loop(&self) {
    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<AgentEvent>(32);

    // Store sender for messaging handler access
    // (passed through handler registration, not stored on self)
    // ... wire event_tx to the message handler setup

    let config = AgentLoopConfig {
        conductor: self.conductor.clone(),
        router: self.router.clone(),
        mcp_registry: self.mcp_registry.clone(),
        agent_memory: self.agent_memory.clone(),
        learning_bus: self.learning_bus.clone(),
        mesh_state: self.mesh_state.clone(),
        compute_budget: self.compute_budget.clone(),
        model_hint: self.model_hint.clone(),
        purpose: self.purpose.clone(),
        orchestrator_tick: self.orchestrator_tick.clone(),
        has_mcp_tools: self.has_mcp_tools,
        tool_loop_budget: self.tool_loop_budget,
        #[cfg(feature = "iroh")]
        iroh_node: self.iroh_node.clone(),
    };

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await; // MCP readiness
        super::agent_loop::run_agent_loop(config, event_rx).await;
    });
}
```

- [ ] **Step 2: Delete the old loop body**

Remove the inline loop code that was at lines ~1339-1728. This is ~400 lines of deletion. The helpers were already moved in Task 7.

- [ ] **Step 3: Wire event_tx to the handler**

The `event_tx` must reach `handle_message_send`. The exact wiring depends on how `A2aServer` currently registers handlers. Check how the RPC handler setup passes references. The sender should be passed through the same dependency injection path as other handler dependencies (conductor, router, etc.), NOT stored as `Option<mpsc::Sender>` on `A2aServer`.

- [ ] **Step 4: Verify compilation**

Run: `cargo build -p arkavo-server -q`
Expected: Compiles.

- [ ] **Step 5: Run existing tests**

Run: `cargo test -p arkavo-server`
Expected: All existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-server/src/server/a2a_server.rs
git commit -m "Replace inline orchestrator loop with agent_loop::run_agent_loop"
```

---

### Task 10: Add event channel to messaging handler

**Files:**
- Modify: `crates/arkavo-server/src/server/handlers/messaging.rs`

- [ ] **Step 1: Add agent_event_tx parameter**

Add to `handle_message_send` signature (after `agent_memory` parameter, line 36):

```rust
agent_event_tx: Option<&tokio::sync::mpsc::Sender<super::super::agent_event::AgentEvent>>,
```

- [ ] **Step 2: Add orchestrator message routing**

Before the existing `tokio::spawn(execute_with_conductor_and_learning(...))` block (around line 261), add:

```rust
if let Some(event_tx) = agent_event_tx {
    use crate::server::agent_event::{AgentEvent, CorrelationId};

    let correlation_id = CorrelationId(uuid::Uuid::new_v4());
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

    // Extract sender DID from request metadata
    let sender_did = request.message.metadata
        .as_ref()
        .and_then(|m| m.get("sender_did"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let content = request.message.parts.iter()
        .filter_map(|p| match p {
            arkavo_protocol::types::MessagePart::Text { content } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let _ = event_tx.send(AgentEvent::IncomingMessage {
        sender: sender_did,
        content,
        task_id: task_id_clone,
        correlation_id,
        reply: reply_tx,
    }).await;

    // Waiter with 120s timeout
    let task_exec = task_executor.clone();
    tokio::spawn(async move {
        match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            reply_rx,
        ).await {
            Ok(Ok(receipt)) => {
                info!(
                    correlation_id = %receipt.correlation_id.0,
                    "Message incorporated into cycle {}",
                    receipt.cycle_id.0,
                );
            }
            Ok(Err(_canceled)) => {
                warn!("Agent loop dropped message sender — updating task to failed");
                // Update task status to failed via task_exec
            }
            Err(_timeout) => {
                warn!("Message not processed within 120s — updating task to timed out");
                // Update task status to timed out via task_exec
            }
        }
    });

    // Return immediately — the event loop will process this message
    // (existing task submission + return logic follows)
} else {
    // Specialist path: existing tokio::spawn(execute_with_conductor_and_learning(...))
    // COMPLETELY UNCHANGED
}
```

- [ ] **Step 3: Update all callers of handle_message_send**

Search for call sites and add the `agent_event_tx` parameter. For specialists (no event channel), pass `None`. For orchestrators, pass `Some(&event_tx)`.

- [ ] **Step 4: Verify compilation**

Run: `cargo build -p arkavo-server -q`
Expected: Compiles.

- [ ] **Step 5: Run existing tests**

Run: `cargo test -p arkavo-server`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-server/src/server/handlers/messaging.rs
git commit -m "Route orchestrator A2A messages through event channel with CycleReceipt"
```

---

### Task 11: Full lint and test verification

**Files:** None (verification only)

- [ ] **Step 1: Format check**

Run: `cargo fmt -- --check`
Expected: No formatting issues.

- [ ] **Step 2: Clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings.

- [ ] **Step 3: Full test suite**

Run: `cargo test -p arkavo-server`
Expected: All tests pass.

- [ ] **Step 4: Build check**

Run: `cargo build -q`
Expected: Full workspace builds cleanly.

- [ ] **Step 5: Commit any fixes**

If any lint/test issues were found, fix and commit:

```bash
git commit -m "Phase 2 lint and test fixes"
```

---

## Phase 3: ToolMemory Cleanup (after overnight validation)

**Gate:** Only proceed after Phase 2 passes overnight RimWorld test confirming conversation persistence works correctly.

### Task 12: Refactor ToolMemory to format_control_signals()

**Files:**
- Modify: `crates/arkavo-server/src/server/tool_memory.rs`

- [ ] **Step 1: Write the failing test**

Add to the test module in `tool_memory.rs`:

```rust
#[spec("SRV-010")]
#[test]
fn test_format_control_signals_none_when_clean() {
    let mem = ToolMemory::new(10);
    // No entries, no warnings — should return None
    assert!(mem.format_control_signals().is_none());
}

#[spec("SRV-010")]
#[test]
fn test_format_control_signals_shows_duplicate_warning() {
    let mut mem = ToolMemory::new(10);
    let args = json!({"query": "wood"});
    mem.add("search".into(), &args, "found 5");
    mem.add("search".into(), &args, "found 5");
    let signals = mem.format_control_signals();
    assert!(signals.is_some());
    assert!(signals.unwrap().contains("already"));
}

#[spec("SRV-010")]
#[test]
fn test_format_control_signals_shows_variety_warning() {
    let mut mem = ToolMemory::new(10);
    for i in 0..4 {
        let args = json!({"x": i});
        mem.add("build".into(), &args, "ok");
    }
    let signals = mem.format_control_signals();
    assert!(signals.is_some());
    // Should mention repeated action type
    assert!(signals.unwrap().to_lowercase().contains("build"));
}

#[spec("SRV-010")]
#[test]
fn test_format_control_signals_shows_setup_state() {
    let mut mem = ToolMemory::new(10);
    mem.add("registerAgent".into(), &json!({}), "ok");
    let signals = mem.format_control_signals();
    assert!(signals.is_some());
    assert!(signals.unwrap().contains("registerAgent"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p arkavo-server format_control_signals`
Expected: FAIL — method does not exist.

- [ ] **Step 3: Implement format_control_signals**

Add to `ToolMemory` impl in `tool_memory.rs`:

```rust
/// Emit only derived control signals — not history replay.
/// Returns None if there are no signals to inject (common case).
pub fn format_control_signals(&self) -> Option<String> {
    let mut signals = Vec::new();

    // Deduplication warnings
    let dupes: Vec<&ToolMemoryEntry> = self.entries.iter()
        .filter(|e| e.is_duplicate)
        .collect();
    if !dupes.is_empty() {
        let mut dedup_section = String::from("## Duplicate Actions Detected\n");
        for d in &dupes {
            use std::fmt::Write;
            let _ = writeln!(dedup_section,
                "- `{}` with same params already called — results were identical",
                d.tool_name,
            );
        }
        signals.push(dedup_section);
    }

    // Action variety warning
    let variety = self.action_variety_warning();
    if !variety.is_empty() {
        signals.push(variety);
    }

    // Setup completion state
    if !self.completed_setup_tools.is_empty() {
        let mut setup = String::from("## Setup State\n");
        for tool in &self.completed_setup_tools {
            use std::fmt::Write;
            let _ = writeln!(setup, "- {tool}: complete (DO NOT call again)");
        }
        signals.push(setup);
    }

    // Error escalation (pattern, not individual errors)
    let error_count = self.entries.iter().filter(|e| e.is_error).count();
    if error_count >= 3 {
        let error_tools: std::collections::HashSet<&str> = self.entries.iter()
            .filter(|e| e.is_error)
            .map(|e| e.tool_name.as_str())
            .collect();
        let mut escalation = format!(
            "## Error Pattern\n{error_count} consecutive errors across: {}. Consider a different approach.",
            error_tools.into_iter().collect::<Vec<_>>().join(", "),
        );
        signals.push(escalation);
    }

    if signals.is_empty() {
        None
    } else {
        Some(signals.join("\n\n"))
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p arkavo-server format_control_signals`
Expected: All 4 tests pass.

- [ ] **Step 5: Remove pending_instructions field**

In `tool_memory.rs`:
- Remove `pending_instructions: VecDeque<String>` from the struct (line 22)
- Remove `pending_instructions: VecDeque::new()` from `new()` (line 48)
- Remove doc comment above the field (lines 19-21)

- [ ] **Step 6: Update agent_loop.rs to use format_control_signals**

In the tick branch of `run_agent_loop`, replace:

```rust
let control_signals = {
    let mem = config.agent_memory.read().await;
    mem.format_for_prompt()
};
```

With:

```rust
let control_signals = {
    let mem = config.agent_memory.read().await;
    mem.format_control_signals().unwrap_or_default()
};
```

- [ ] **Step 7: Verify compilation and tests**

Run: `cargo build -p arkavo-server -q && cargo test -p arkavo-server`
Expected: Compiles and all tests pass.

- [ ] **Step 8: Clippy check**

Run: `cargo clippy -- -D warnings`
Expected: No warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/arkavo-server/src/server/tool_memory.rs crates/arkavo-server/src/server/agent_loop.rs
git commit -m "ToolMemory: control-signals-only injection, remove pending_instructions"
```

---

## Behavioral Preservation Checklist

After Phase 2, verify each behavior from the original loop:

- [ ] Adaptive interval backoff (5/15/30/60s based on consecutive_timeouts)
- [ ] Prompt hash deduplication (skip except every 5th duplicate)
- [ ] Dead-man's switch (warnings at 3+, context reset at 8+)
- [ ] State broadcast to specialists every 3 cycles
- [ ] Budget gate for specialists only (commanders never gated)
- [ ] Cycle counter broadcast to orchestrator_tick atomic
- [ ] Setup tool optimization (via ToolMemory.is_setup_complete)
- [ ] Learning bus fast-path lesson on degenerate reset
- [ ] A2A messages reach MCP tool pipeline (Bug 1 fix)
- [ ] Conversation context persists across cycles (Bug 2 fix)
- [ ] All inputs process through single event queue (Bug 3 fix)
