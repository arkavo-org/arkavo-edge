# Unified Agent Loop Refactor

## Problem

Three critical bugs discovered during RimWorld Gemma 4 testing stem from the orchestrator agent loop and A2A message handling being two separate paths through the conductor:

**Bug 1: A2A messages can't execute MCP tools.** `handle_message_send` spawns a separate `execute_with_conductor_and_learning` that races for the GPU and completes with a text response (no tool calls). External commands like "reset the colony" never reach the orchestrator's tool-calling pipeline.

**Bug 2: Conversation context resets every cycle.** `conductor.rs:393` does `let mut messages = Vec::new()` each call. The planner accumulates context within a cycle (rounds 0-2) but loses ALL history between cycles. The model forgets registerAgent results, observe data, etc.

**Bug 3: No unified event processing.** Timer ticks, A2A messages, specialist completions, and human overrides are handled via different mechanisms stitched together at the top of each cycle.

## Architecture

Replace the inline `loop { sleep(interval); execute_cycle() }` in `a2a_server.rs:1339-1728` with a `tokio::select!` event loop that processes all inputs through a single queue with persistent conversation context.

```
AgentEvent::IncomingMessage -+
AgentEvent::HumanOverride   -+-> run_agent_loop() -> ConversationWindow -> Planner (3-track)
AgentEvent::Shutdown         -+       |
                                      +-> ToolMemory (control signals only)
```

## Phased Implementation

**Phase 1** (pure additive, zero risk): `agent_event.rs`, `conversation_window.rs`, `token_estimator.rs` as standalone modules with unit tests. No integration.

**Phase 2** (high-risk, overnight test gate): Replace loop body in `a2a_server.rs` with `tokio::select!` loop wiring `agent_event_rx`. Keep ToolMemory's current `format_for_prompt()` temporarily alongside ConversationWindow -- both emit into the prompt. The duplication is ugly but safe. If ConversationWindow has a trimming bug or the TokenEstimator underestimates, ToolMemory's summaries act as fallback. RimWorld overnight test validates Phase 2 before proceeding.

**Phase 3** (cleanup, low risk): Refactor ToolMemory to `format_control_signals()`, remove `pending_instructions`, remove history-replay sections. Only after Phase 2 is stable.

## Module: Event Types (`agent_event.rs`)

### Types

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CycleId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CorrelationId(pub uuid::Uuid);

pub enum MessageDisposition {
    /// Included in the current cycle's prompt
    Incorporated { cycle_id: CycleId },
    /// Queued for next cycle (current cycle was already assembling)
    Deferred,
    /// Rejected (budget exceeded, agent shutting down, etc.)
    Rejected { reason: String },
}

pub struct CycleReceipt {
    pub cycle_id: CycleId,
    pub correlation_id: CorrelationId,
    pub disposition: MessageDisposition,
}

pub enum AgentEvent {
    IncomingMessage {
        sender: String,  // did:key of the sending agent
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

### Design Decisions

- `CycleId` and `CorrelationId` are newtypes with explicit `Copy` derive. `uuid::Uuid` is `Copy`, so this is free. Prevents downstream code from accidentally taking `&CorrelationId` references.
- `HumanOverride` is a separate variant (not a flag on `IncomingMessage`) because it has distinct priority semantics (`insert(0, ...)` vs `push`).
- `IncomingMessage` carries `sender: String` (DID) for three purposes: cycle prompt annotation (`[msg correlation_id=X from=did:key:Y]`), learning system attribution, and future trust chain filtering.
- `PendingMessage.reply` is `Option` -- consumed with `.take()` when sending the receipt. `None` handles cases where receipt was already sent or sender dropped.
- `PendingMessage` uses `priority: MessagePriority` enum instead of `is_human_override: bool`. Extensible for future priority tiers (e.g., self-directed instructions).
- `Shutdown` has no reply channel -- fire-and-forget.

### CycleReceipt Semantics

The oneshot channel's job is to close the synchronization gap between "message submitted" and "message entered the processing pipeline." Everything after that -- did the LLM address it, did a tool call succeed -- flows through the task status bus and learning system.

Properties:
- Senders can distinguish "not yet" (Deferred) from "never" (Rejected) for retry logic.
- `CorrelationId` bridges the sync/async boundary. Sender takes the ID, subscribes to task status events filtered by that ID.
- The oneshot resolves fast -- bounded by event loop latency, not inference latency. Critical for gossip-originated messages.

### Correlation ID Propagation

`CorrelationId` propagates into the cycle prompt as a structured annotation: `[msg correlation_id=X from=did:key:Y] content here`. This makes it possible to observe when the LLM naturally references or addresses a specific message, giving the learning system a free attribution signal without requiring per-message decomposition. The orchestrator cycle is a batch cognitive step -- trying to reverse-engineer intent attribution from LLM output would be fragile.

## Module: Token Estimation (`token_estimator.rs`)

### Trait

```rust
pub trait TokenEstimator: Send + Sync {
    fn estimate_tokens(&self, text: &str) -> usize;

    /// Counting-only fast path. Implementors can skip allocating
    /// a full token vector. Default: delegates to estimate_tokens.
    fn estimate_token_count(&self, text: &str) -> usize {
        self.estimate_tokens(text)
    }
}
```

### Implementation: LlamaTokenEstimator (primary)

Wrap the already-loaded model's tokenizer via llama.cpp FFI:

```rust
pub struct LlamaTokenEstimator {
    model: Arc<LlamaModel>,
}

impl TokenEstimator for LlamaTokenEstimator {
    fn estimate_token_count(&self, text: &str) -> usize {
        // llama_tokenize() is a pure vocabulary lookup — no GPU, no inference
        self.model.tokenize(text, false).len()
    }
}
```

**Construction**: At agent startup, after model loading but before the event loop starts, grab a reference to any loaded model. The tokenizer is already in memory — zero additional binary cost.

**Why this works**: The original argument against querying the router was about the routing *decision* creating a circular dependency. Tokenization isn't routing. We don't need to know which model *will be selected* — we need *any* loaded model's vocabulary to count tokens, and they're all close enough for a budget gate with 30% headroom.

### Implementation: MockEstimator (testing)

```rust
pub struct MockEstimator;

impl TokenEstimator for MockEstimator {
    fn estimate_token_count(&self, text: &str) -> usize {
        text.len() / 4  // chars/4, deterministic for test assertions
    }
}
```

### Design Decisions

- **Not a vendored BPE merge table**: Every agent already has a tokenizer in memory via the loaded llama.cpp model. Vendoring a separate 800KB merge table duplicates what's already there.
- **Not tiktoken-rs or `tokenizers` crate**: Unnecessary external dependency when llama.cpp's tokenizer is already linked.
- **Budget gate, not billing meter**: Trimming decisions tolerate 5-10% estimation error because the 70% budget has 30% headroom. The actual provider token count (`n_prompt_eval` from llama.cpp) remains ground truth for `context_utilization_pct` after inference.
- **`estimate_token_count()` method**: ConversationWindow only needs the count, never token IDs. The `LlamaTokenEstimator` counts without returning the full token vector.
- **Mock estimator for tests**: `chars / 4` is deterministic. Tests verify trimming behavior at boundary conditions, not exact token counts.
- **Trait preserves future WASM path**: When Vokra moves to WASM, the `LlamaTokenEstimator` won't be available. A self-contained BPE implementation can slot into the trait at that point without changing ConversationWindow.

## Module: Conversation Window (`conversation_window.rs`)

### Structure

```rust
pub struct ConversationWindow {
    system_message: Option<arkavo_llm::Message>,
    history: VecDeque<arkavo_llm::Message>,
    history_tokens: usize,  // running total
    max_history_tokens: usize,
    estimator: Arc<dyn TokenEstimator>,
}
```

### API

```rust
impl ConversationWindow {
    pub fn new(
        min_feasible_context: usize,
        estimator: Arc<dyn TokenEstimator>,
    ) -> Self;

    pub fn set_system_message(&mut self, msg: arkavo_llm::Message);

    /// Append and trim. O(1) amortized via VecDeque + running token total.
    pub fn push(&mut self, msg: arkavo_llm::Message);

    /// Returns [system + optional suffix, history]. Does NOT include
    /// the current cycle's user message -- caller pushes that to history
    /// before calling the conductor.
    pub fn build_messages(
        &self,
        system_suffix: Option<&str>,
    ) -> Vec<arkavo_llm::Message>;

    /// Hard reset after degenerate output detection.
    pub fn clear_history(&mut self);

    pub fn history_len(&self) -> usize;

    /// Accessor for dynamic recomputation path (static for now).
    fn max_history_tokens(&self) -> usize;
}
```

### Trimming Implementation

```rust
fn push(&mut self, msg: arkavo_llm::Message) {
    self.history_tokens += self.estimator.estimate_token_count(msg.content());
    self.history.push_back(msg);
    while self.history.len() > 2 && self.history_tokens > self.max_history_tokens() {
        if let Some(oldest) = self.history.pop_front() {
            self.history_tokens -= self.estimator.estimate_token_count(oldest.content());
        }
    }
}
```

### Design Decisions

- **Single owner, no `Arc<RwLock>`**: Owned exclusively by the event loop task. Only one writer.
- **`VecDeque` + running token total**: O(1) push/pop, no re-tokenization. A `Vec` with `remove(0)` is O(n), and re-tokenizing all messages on every push is O(n^2) -- both surface at message 500+ in overnight runs.
- **`min_feasible_context` at construction**: 70% of the minimum context size across currently-feasible model candidates. Uses the minimum, not maximum, because the router could select a smaller-context model.
- **Static `max_history_tokens` accessed via method**: Accepts staleness for now. `max_history_tokens()` method accessor means dynamic recomputation is a one-line change later.
- **`min_feasible_context_size` requires a new Router method**: The Router doesn't currently expose context sizes. Add `pub fn min_feasible_context_size(&self) -> usize` to `arkavo_router::Router` that queries loaded providers via `model.get_trained_context_size()` (available on `LlamaModel`) and returns the minimum. If no models are loaded yet, return a conservative default (4096). This is a Phase 1 addition alongside ConversationWindow.
- **`build_messages` returns `[system, history]` only**: The current cycle's user message is NOT included. The caller pushes the user message to history unconditionally before calling the conductor, then conditionally pushes the assistant response on success. Failed cycle prompts stay in history as valid context.
- **`system_suffix: Option<&str>` not `control_signals`**: ConversationWindow doesn't know about ToolMemory control signals. It appends arbitrary text to the system message per-cycle without altering the stored system message. The "fixed, never trimmed" invariant is preserved.
- **Minimum 2 messages kept**: Last user+assistant pair always retained, even if over token budget.
- **`clear_history` resets `history_tokens` to 0**: Used by degenerate output reset (8+ no-action cycles).

### Relationship to ToolMemory

**ConversationWindow is "what happened."** Raw conversation history -- the model's own prior outputs.

**ToolMemory is "what the agent should know about what happened that isn't obvious from reading the history."** Derived control signals only.

In Phase 2, both emit into the prompt (ugly but safe fallback). In Phase 3, ToolMemory stops restating history and becomes purely a control signal layer via `format_control_signals()`.

## Module: Agent Loop (`agent_loop.rs`)

### Entry Point

```rust
pub struct AgentLoopConfig {
    pub conductor: Arc<Conductor<InMemoryTaskStore>>,
    pub router: Arc<arkavo_router::Router>,
    pub mcp_registry: Arc<McpRegistry>,
    pub agent_memory: Arc<RwLock<ToolMemory>>,
    pub learning_bus: Option<Arc<LearningBus>>,
    pub mesh_state: Arc<arkavo_mcp_mesh::MeshToolsState>,
    pub compute_budget: arkavo_budget::SharedComputeBudget,
    pub model_hint: Option<arkavo_router::ModelChoice>,
    pub purpose: String,
    pub orchestrator_tick: Arc<AtomicU64>,
    pub has_mcp_tools: bool,
    pub tool_loop_budget: Option<u32>,
    #[cfg(feature = "iroh")]
    pub iroh_node: Option<Arc<arkavo_tdf_iroh::IrohNode>>,
}

pub async fn run_agent_loop(
    config: AgentLoopConfig,
    agent_event_rx: mpsc::Receiver<AgentEvent>,
) {
```

`agent_event_rx` is separate from `AgentLoopConfig` because it's consumed (moved into the loop), while config is a bag of shared references.

### Event Loop Structure

```rust
loop {
    tokio::select! {
        _ = tick_interval.tick() => {
            // === CYCLE EXECUTION ===
            // 1. Budget gate (specialists only)
            // 2. Drain gossip completions -> specialist_context
            // 3. Drain pending_messages -> message annotations + send CycleReceipts
            // 4. Build control signals from ToolMemory
            // 5. Assemble cycle prompt
            // 6. Duplicate prompt detection (hash + skip)
            // 7. Push user message to ConversationWindow
            // 8. Build messages via ConversationWindow
            // 9. Execute via conductor (with existing_messages)
            // 10. Push assistant response (on success only)
            // 11. Update timeout/action tracking
            // 12. Degenerate reset (8+ no-action cycles -> clear both)
            // 13. State broadcast every 3 cycles
            // 14. Update adaptive interval (only when changed)
        }
        Some(event) = agent_event_rx.recv() => {
            // Route to pending_messages with priority
            // tick_interval.reset() for fast processing
        }
    }
}
```

### Two-Stage Message Buffer

Messages have a two-stage buffer design:

- **Arrival buffer** (channel): `agent_event_rx` accumulates messages that arrive during execution. The `tokio::select!` only runs one branch at a time -- while the tick branch executes (potentially 30+ seconds of inference), incoming messages queue here.
- **Staging buffer** (`pending_messages: Vec<PendingMessage>`): Drained from the arrival buffer in the `recv` branch, consumed by the tick branch at step 3. This is the set of messages incorporated into the current cycle.

Do not drain the channel inside the tick branch to "pick up late arrivals" -- the two-stage design is intentional.

### Persistent State (migrated from a2a_server.rs:1352-1357)

```rust
let mut cycle: u64 = 0;
let mut consecutive_no_action_cycles: u32 = 0;
let mut last_broadcast_cycle: u64 = 0;
let mut consecutive_timeouts: u32 = 0;
let mut last_cycle_prompt_hash: u64 = 0;
let mut consecutive_duplicate_prompts: u32 = 0;
```

Plus new state:
- `conversation: ConversationWindow` -- persistent history
- `pending_messages: Vec<PendingMessage>` -- staging buffer
- `cycle_interval_secs: u64` -- current interval for change detection

### Message Push Sequence

User message is pushed unconditionally before the conductor call. Assistant response is pushed conditionally on success:

```rust
// Always record what we asked (failed prompts are still valid context)
let user_msg = arkavo_llm::Message::user(&cycle_prompt);
conversation.push(user_msg);

// Build messages from [system + suffix, history] -- history now includes user_msg
let messages = conversation.build_messages(Some(&control_signals));

// Execute conductor with pre-built messages
let result = execute_with_conductor_and_learning(..., Some(messages)).await;

match result {
    Ok(ref tool_result) => {
        conversation.push(arkavo_llm::Message::assistant(&tool_result.final_text));
        // update metrics...
    }
    Err(ref e) => {
        // Don't push assistant message -- there was no response
        consecutive_timeouts += 1;
    }
}
```

### Adaptive Interval

Only recreate the interval when the computed value changes:

```rust
let new_interval_secs = match consecutive_timeouts {
    0 => 5, 1 => 15, 2 => 30, _ => 60,
};
if new_interval_secs != cycle_interval_secs {
    cycle_interval_secs = new_interval_secs;
    tick_interval = tokio::time::interval(Duration::from_secs(cycle_interval_secs));
    tick_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
}
```

### Behavioral Preservation Checklist

All behaviors from the current inline loop must be preserved:
- [ ] Adaptive interval backoff (5/15/30/60s based on consecutive_timeouts)
- [ ] Prompt hash deduplication (skip except every 5th duplicate)
- [ ] Dead-man's switch (warnings at 3+, context reset at 8+)
- [ ] State broadcast to specialists every 3 cycles
- [ ] Budget gate for specialists only (commanders never gated)
- [ ] Cycle counter broadcast to orchestrator_tick atomic
- [ ] Setup tool optimization (via ToolMemory.is_setup_complete)
- [ ] Learning bus fast-path lesson on degenerate reset

### Helper Functions (migrated from a2a_server.rs)

All helpers are pure functions of their arguments -- none need `&self` on `A2aServer`:

| Function | Source | Purpose |
|----------|--------|---------|
| `detect_urgency()` | a2a_server.rs:1819-1846 | Keyword frequency -> urgency tier |
| `compact_observation()` | a2a_server.rs:1853-1874 | Prefer Delta section, truncate to 2000 chars |
| `compute_per_agent_bytes_static()` | a2a_server.rs:1882-1895 | Memory budget per specialist |
| `broadcast_state_to_peers()` | a2a_server.rs:1974-2061 | Proactive analysis tasks to specialists |
| `send_advisory_task()` | a2a_server.rs:2067+ | Delegation RPC to specialist. Needs only `mesh_state` + protocol types. No `TaskExecutor`. |
| `stage_on_iroh()` | a2a_server.rs:1951-1972 | P2P staging for large data (feature-gated) |
| `drain_specialist_completions()` | NEW | Extract from a2a_server.rs:1404-1466 |
| `drain_pending_messages()` | NEW | Build message block + CycleReceipts |
| `assemble_cycle_prompt()` | NEW | Extract from a2a_server.rs:1468-1504 |
| `should_skip_duplicate()` | NEW | Extract from a2a_server.rs:1506-1542 |
| `update_cycle_metrics()` | NEW | Extract from a2a_server.rs:1656-1701 |

## Modifications to Existing Files

### conductor.rs (+15 lines)

Add `existing_messages: Option<Vec<arkavo_llm::Message>>` parameter to `execute_with_conductor_and_learning`.

```rust
let messages = if let Some(existing) = existing_messages {
    existing
} else {
    // Original construction -- backward compat
    let mut messages = Vec::new();
    // ... system + user (unchanged)
    messages
};
```

All existing callers pass `None`. Zero behavioral change for specialists, messaging handler, or any other call site.

### handlers/messaging.rs (+25 lines)

Add `agent_event_tx: Option<mpsc::Sender<AgentEvent>>` parameter to `handle_message_send`. Passed through handler registration, not stored on `A2aServer`.

For orchestrators (agents with an event channel):

```rust
if let Some(ref event_tx) = agent_event_tx {
    let correlation_id = CorrelationId(uuid::Uuid::new_v4());
    let (reply_tx, reply_rx) = oneshot::channel();
    let sender_did = extract_sender_did(&request);

    event_tx.send(AgentEvent::IncomingMessage {
        sender: sender_did,
        content: extract_content(&request),
        task_id,
        correlation_id,
        reply: reply_tx,
    }).await.map_err(|_| /* channel closed */)?;

    // Waiter with 120s timeout (2x max cycle interval + inference headroom)
    tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_secs(120), reply_rx).await {
            Ok(Ok(receipt)) => {
                // Update task with correlation_id for tracking
            }
            Ok(Err(_canceled)) => {
                // Agent loop dropped sender -- update task status to failed
            }
            Err(_timeout) => {
                // 2 minutes without processing -- update task status to timed out
            }
        }
    });
} else {
    // Specialist path: existing tokio::spawn(execute_with_conductor_and_learning(...))
    // Completely unchanged
}
```

### a2a_server.rs (-400/+40 lines)

In `start_orchestrator_loop()`:

```rust
let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(32);

// Pass sender through handler registration (not stored on self)
self.register_message_handler(event_tx.clone());

let config = AgentLoopConfig { /* ... from current closure captures */ };

tokio::spawn(async move {
    tokio::time::sleep(Duration::from_secs(3)).await; // MCP readiness
    agent_loop::run_agent_loop(config, event_rx).await;
});
```

The entire inline loop body (lines ~1352-1728) is deleted. All helper functions move to `agent_loop.rs`.

### tool_memory.rs (Phase 3 only)

- Remove `pending_instructions: VecDeque<String>` field (dead weight -- never written to or read from)
- Refactor `format_for_prompt()` to `format_control_signals() -> Option<String>`
- Control signals emit only: dedup warnings, action variety, setup state, error escalation
- Stop emitting: "Recent Actions" lists, "Already Completed" sections, history replay
- ToolMemory never references conversation content -- only its own counters, dedup keys, state flags
- Control signals come from system prompt token budget, not history budget

### mod.rs (+5 lines)

```rust
mod agent_event;
mod agent_loop;
mod conversation_window;
mod token_estimator;
```

Re-export `AgentEvent`, `AgentLoopConfig`, `CycleReceipt`, `CorrelationId` for the messaging handler.

## What Stays the Same

- Three-track planner (`conductor_parallel.rs`) -- unchanged, receives pre-built messages
- ToolMemory sliding window, dedup, setup tracking -- unchanged (Phase 3 changes prompt injection only)
- LearningBus -- same gossip transport, drain_task_completions, episode synthesis
- ComputeBudget -- same gating logic
- MCP bridge -- unchanged
- Specialist behavior -- specialists keep current `tokio::spawn(execute_with_conductor_and_learning)` path
- Tool extraction, tool parsing, model format detection -- unchanged
- Decision routing, Thompson Sampling, fallback chains -- unchanged

## Files Changed

| File | Action | Phase | Lines |
|------|--------|-------|-------|
| `server/agent_event.rs` | NEW | 1 | ~80 |
| `server/token_estimator.rs` | NEW | 1 | ~120 |
| `server/conversation_window.rs` | NEW | 1 | ~120 |
| `server/agent_loop.rs` | NEW | 2 | ~300 |
| `server/a2a_server.rs` | MODIFY | 2 | -400/+40 |
| `server/conductor.rs` | MODIFY | 2 | +15 |
| `server/handlers/messaging.rs` | MODIFY | 2 | +25 |
| `server/mod.rs` | MODIFY | 1+2 | +8 |
| `server/tool_memory.rs` | MODIFY | 3 | ~-80/+40 |

## Verification

### Phase 1

- `cargo build -q` -- compiles
- `cargo clippy -- -D warnings` -- no warnings
- `cargo test -p arkavo-server` -- all existing tests pass
- Unit tests for ConversationWindow trimming at boundary conditions
- Unit tests for BPE TokenEstimator against known token counts
- Unit tests for CycleReceipt/MessageDisposition serialization

### Phase 2

- All Phase 1 checks pass
- Launch RimWorld example with Gemma 4 26B-A4B
- Verify: commander calls registerAgent -> observe -> step in sequence with context carried across cycles
- Send "reset the colony" via UI chat -> verify episodeSummary + reset are called (not text-only response)
- Verify specialists still respond to A2A messages via existing path
- Verify conversation window trims correctly (check context_tokens in telemetry)
- Overnight RimWorld test gate before Phase 3

### Phase 3

- All Phase 2 checks pass
- Verify no duplicate information in prompts (ToolMemory control signals + ConversationWindow history)
- Verify ToolMemory silent when no control signals needed
- Verify `pending_instructions` field removal causes no compilation errors
