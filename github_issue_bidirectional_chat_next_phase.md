# [EPIC] Bidirectional Chat Protocol - Production Hardening & UX Completion

## Overview

The bidirectional chat protocol foundation has been successfully implemented with proper session lifecycle management (`chat_open/send/stream/close`). This epic covers the next phase: production hardening, UX completion, and operational readiness.

## Health Check Status

| Area | Status | Notes |
|------|--------|-------|
| Build | ✅ | Compiles cleanly (minor warnings to fix) |
| Session lifecycle | ✅ | Solid API with deterministic ordering |
| Multi-UI fan-out | ✅ | Broadcast channels support dashboards + mobile |
| Back-pressure | ⚠️ | Minimal (bounded mpsc only at gateway→WS) |
| Error propagation | ⚠️ | Works but UI can't distinguish error types |
| Auth / ACL | ❓ | Not yet implemented |
| Telemetry | ❓ | No per-session tracing hooks |
| Persistence | ❓ | In-memory only (no TTL/encrypted store) |

## Milestone Structure

### M1: UX Complete (~2 days)
**Goal**: Users can have multi-turn conversations with proper error handling

**Tasks**:
- [ ] **Code hygiene** - Fix warnings and unused imports (#1)
- [ ] **Wire UI send path** - Front-end can emit follow-up messages (#2)  
- [ ] **Structured error handling** - UI distinguishes error types (#3)

**Acceptance Criteria**:
- ✅ Build with zero warnings
- ✅ User can send multiple messages in same session
- ✅ UI shows appropriate error states (retryable vs fatal)

### M2: Secure & Observable (~1 week)
**Goal**: Production-ready security and monitoring

**Tasks**:
- [ ] **Session authentication** - JWT-based access control (#4)
- [ ] **Observability stack** - Tracing spans and metrics (#5)
- [ ] **Tool-call delta support** - Future-proof protocol (#6)

**Acceptance Criteria**:
- ✅ Unauthorized users cannot access agent sessions  
- ✅ End-to-end latency and token/sec metrics available
- ✅ Tool calls stream properly to UI

### M3: Load-Ready (~2 weeks)
**Goal**: Handle production load with proper resource management

**Tasks**:
- [ ] **Back-pressure management** - Prevent resource exhaustion (#7)
- [ ] **Persistence strategy** - Optional encrypted storage (#8)
- [ ] **Integration test suite** - Multi-turn conversation testing (#9)
- [ ] **Documentation** - Protocol diagrams and examples (#10)

**Acceptance Criteria**:
- ✅ System gracefully handles slow consumers
- ✅ Sessions persist across server restarts (if enabled)
- ✅ Comprehensive test coverage for edge cases

## Detailed Task Breakdown

### #1 Code Hygiene (M1)
```bash
# Fix compilation warnings
- Remove unused `StreamEndReason` import in server.rs
- Clean up unused variables (_broadcast_tx, _trace_id, etc.)
- Add #[allow(dead_code)] or remove unused struct fields
```

### #2 Wire UI Send Path (M1)
```typescript
// In messages.ts - Add chat_send RPC call
async function sendMessage(sessionId: string, text: string) {
  await client.request('chat_send', [sessionId, { content: text }]);
}

// Track session state
const sessionState = new Map<string, { id: string, lastSeq: number }>();
```

### #3 Structured Error Handling (M1)
```rust
// Extend MessageDeltaContent with error kinds
pub enum ErrorKind {
    Recoverable,    // Network timeout, retry
    Fatal,          // Model error, abort session  
    RateLimited,    // Back off and retry
    Unauthorized,   // Auth failure
}

pub struct ErrorDelta {
    pub kind: ErrorKind,
    pub code: String,
    pub message: String,
    pub retry_after_ms: Option<u64>,
}
```

### #4 Session Authentication (M2)
```rust
// Add auth to ChatOpenRequest
pub struct ChatOpenRequest {
    pub auth_token: Option<String>, // JWT
    pub context: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

// Validate JWT in chat_open handler
async fn chat_open(&self, request: ChatOpenRequest) -> RpcResult<ChatSession> {
    let claims = validate_jwt(&request.auth_token?)?;
    // Check scopes/permissions...
}
```

### #5 Observability (M2)
```rust
// Add tracing spans
#[tracing::instrument]
async fn chat_send(&self, session_id: String, message: UserMessage) -> RpcResult<()> {
    histogram!("chat_turn_latency").record(start.elapsed());
    counter!("messages_sent_total").increment(1);
}

// Per-session metrics
struct SessionMetrics {
    tokens_per_second: f64,
    avg_response_time: Duration,
    error_rate: f64,
}
```

### #6 Tool Call Support (M2)
```rust
// Already defined in types.rs, ensure UI handles it
MessageDeltaContent::ToolCall { 
    tool_call_id: String,
    delta: String, // JSON fragments
}
```

### #7 Back-pressure Management (M3)
```rust
// Add flow control constants
const MAX_PENDING_TOKENS: usize = 1000;
const MAX_PENDING_DELTAS: usize = 50;

// New delta type for backpressure
MessageDeltaContent::BackpressureWarning {
    reason: "slow_consumer" | "rate_limited",
    retry_after_ms: u64,
}
```

### #8 Persistence Strategy (M3)
```rust
// Optional encrypted session storage
pub enum PersistenceMode {
    Ephemeral,          // Memory only (current)
    Encrypted(PathBuf), // SQLite with encryption
}

impl ChatSessionManager {
    pub async fn export_session(&self, session_id: &str) -> Result<Vec<u8>>;
    pub async fn import_session(&self, data: &[u8]) -> Result<String>;
}
```

### #9 Integration Tests (M3)
```rust
#[tokio::test]
async fn test_multi_turn_conversation() {
    let server = MockA2aServer::new();
    let client = connect_to_server().await;
    
    // Open session
    let session = client.request("chat_open", [{}]).await?;
    
    // Send first message
    client.request("chat_send", [session.session_id, "Hello"]).await?;
    
    // Verify streaming response
    let deltas = collect_deltas(&client, &session.session_id).await;
    assert!(deltas.len() > 0);
    
    // Send follow-up message
    client.request("chat_send", [session.session_id, "Tell me more"]).await?;
    
    // Verify continued streaming
    let more_deltas = collect_deltas(&client, &session.session_id).await;
    assert!(more_deltas[0].sequence > deltas.last().unwrap().sequence);
}
```

### #10 Documentation (M3)
```markdown
docs/protocol/chat-v2.md:
- Sequence diagrams for session lifecycle
- Error handling flowcharts  
- Authentication examples
- Performance tuning guide

README.md updates:
- Multi-turn conversation example
- Configuration options
- Monitoring setup
```

## Success Metrics

**M1 Success**: 
- Zero build warnings
- UI demos show multi-turn conversations
- Error scenarios display appropriate user feedback

**M2 Success**:
- Security audit passes (JWT validation, ACL)
- Monitoring dashboard shows real-time metrics
- Tool calls work end-to-end

**M3 Success**:
- Load testing with 100 concurrent sessions
- No memory leaks over 24h run
- Session recovery after server restart

## Priority Rationale

1. **M1 first** enables UX designers to polish the chat interface while engineering hardens the backend
2. **M2 security** is required before external deployments  
3. **M3 load-readiness** ensures production scaling without architectural changes

## Related Files

- `crates/arkavo-protocol/src/server.rs` - RPC implementations
- `crates/arkavo-protocol/src/chat_session.rs` - Session management
- `crates/arkavo-agui/src/agent_connection.rs` - Client-side session handling
- `crates/arkavo-agui/src/types.rs` - UI event types
- `docs/longterm-memory.md` - Architecture decisions

---

**Assignee**: Development Team  
**Labels**: `epic`, `bidirectional-chat`, `production-ready`  
**Sprint**: Next 3 sprints (M1 → M2 → M3)