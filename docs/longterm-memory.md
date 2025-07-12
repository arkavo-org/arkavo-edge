# Long-Term Memory

## Agent LLM Configuration and Streaming Chat Implementation

### Model URL Format (AGENTS.md)
The expected format for model URLs in AGENTS.md is:
```
provider://host:port/model
```

Currently only Ollama is supported:
```
ollama://10.0.0.101:11434/devstral:latest
```

### Key Architecture Insights

1. **AG-UI Gateway vs Agent Roles**:
   - AG-UI gateway is itself an AI agent that orchestrates other agents
   - Each headless agent manages its own LLM connection based on AGENTS.md config
   - Gateway should NOT have its own LLM - it forwards to agents

2. **Critical Environment Variable Fix**:
   - Ollama client expects `OLLAMA_BASE_URL` not `OLLAMA_URL`
   - This was causing "HTTP request failed: error sending request" errors

3. **Bidirectional Chat Protocol Architecture** (✅ COMPLETED):
   - **Full-duplex communication** via session-based RPC methods
   - **Session Lifecycle**: chat_open → chat_send (multiple) → chat_stream (subscribe) → chat_close
   - **Multi-UI Support**: Broadcast channels allow multiple UIs per session
   - **Context Management**: Conversation history maintained per session
   - **Resource Management**: Proper cleanup with Drop traits and abort signals

4. **Error Propagation**:
   - Errors from agent LLM connections now properly propagate to UI
   - Users see actual error messages instead of generic failures

5. **Resource Management**:
   - Removed local LLM adapter from AG-UI gateway
   - All LLM connections managed by individual agents
   - Proper cleanup on agent disconnect with terminal delta notification

### Implementation Status
- ✅ StreamLlmModel abstraction with typed deltas
- ✅ Agent-based LLM configuration from AGENTS.md
- ✅ Error propagation to UI
- ✅ Resource cleanup on agent crash
- ✅ Ordered message delivery with sequence numbers
- ✅ **Bidirectional chat protocol (COMPLETED)**
  - chat_open/send/stream/close RPC methods
  - Session-based communication with multi-UI broadcast
  - Proper LLM context management per session
- ⚠️  UI send path integration (partial - needs messages.ts wiring)
- ⚠️  Back-pressure management (basic bounded channels only)
- ⚠️  Structured error handling (strings only, no error types)
- ❌ Session persistence across reconnects
- ❌ Authentication layer (JWT tokens)
- ❌ Observability (tracing spans, metrics)

### Common Issues and Solutions
1. **"Failed to start LLM stream: HTTP request failed"**
   - Check Ollama is running at configured address
   - Verify model exists: `curl http://host:port/api/tags`
   - Ensure OLLAMA_BASE_URL is set correctly (not OLLAMA_URL)

2. **"Message sending through subscription not yet implemented"**
   - ✅ **RESOLVED**: Implemented proper bidirectional protocol
   - Now uses session-based chat_send RPC method
   - No more "new subscription per message" workaround

3. **Next Phase Priority Issues**
   - **UI Integration**: Frontend can receive deltas but not send follow-up messages
   - **Back-pressure**: No throttling on agent→gateway leg under load
   - **Error UX**: UI cannot distinguish retryable vs fatal errors
   - **Security**: No authentication/authorization layer
   - **Observability**: Missing per-session tracing and metrics

## Bidirectional Chat Protocol - Implementation Summary

### What Was Accomplished

The bidirectional chat protocol implementation successfully replaced the previous half-duplex workaround with a proper full-duplex communication system:

#### Core Protocol Changes
- **New RPC Methods**: `chat_open`, `chat_send`, `chat_close`, `chat_stream`
- **Session Management**: `ChatSessionManager` with proper lifecycle handling
- **Message Types**: Enhanced `MessageDelta` with session_id and sequence fields
- **Error Handling**: Structured error propagation through the protocol stack

#### Key Technical Achievements
1. **Session-Based Architecture**: Single session supports multiple user messages with streaming responses
2. **Multi-UI Broadcast**: Multiple UIs can subscribe to the same session via broadcast channels
3. **Context Preservation**: Conversation history maintained per session for better LLM responses
4. **Resource Management**: Proper cleanup prevents memory leaks and zombie sessions
5. **Ordered Delivery**: Sequence numbers ensure deterministic message ordering

#### Files Modified
- `crates/arkavo-protocol/src/server.rs` - RPC method implementations
- `crates/arkavo-protocol/src/chat_session.rs` - Session management logic
- `crates/arkavo-protocol/src/types.rs` - Protocol type definitions
- `crates/arkavo-agui/src/agent_connection.rs` - Client-side session handling

### Migration Impact

**Before**: Each user message created a new subscription
```rust
// Old workaround - wasteful
client.subscribe("chat_subscribe", message) // New subscription each time
```

**After**: Session-based communication
```rust
// New protocol - efficient
let session = client.request("chat_open", {}).await;
client.request("chat_send", [session.session_id, message]).await; // Reuse session
```

### Next Phase Roadmap

The foundation is now solid for production hardening:

1. **M1 (UX Complete)**: Wire UI send path, fix warnings, structured errors
2. **M2 (Secure & Observable)**: JWT auth, tracing spans, metrics
3. **M3 (Load-Ready)**: Back-pressure, persistence, integration tests

See `github_issue_bidirectional_chat_next_phase.md` for detailed implementation plan.