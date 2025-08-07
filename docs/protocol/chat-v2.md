# Bidirectional Chat Protocol v2

## Overview

The Bidirectional Chat Protocol v2 provides secure, observable, and load-ready chat sessions with real-time streaming, authentication, and back-pressure management. This protocol builds upon the A2A (Agent-to-Agent) foundation to deliver production-ready chat capabilities.

## Features

### M2: Secure & Observable

#### Session Authentication
- **JWT Support**: Sessions can be authenticated using signed JWT tokens
- **Pluggable Auth Backend**: Support for multiple authentication methods via the `AuthBackend` trait
- **Session Context**: User identity (`sub`) and scopes are stored in session state
- **Tool Call Authorization**: `x-session-user` header exposed in tool calls for authorization

#### Tool-call Delta Support
- **Streaming Tool Calls**: Tool calls are streamed as deltas alongside text content
- **ToolCallDelta Structure**:
  ```rust
  ToolCall {
      tool_call_id: String,
      name: Option<String>,        // Only sent on first delta
      args_json_fragment: String,   // JSON fragment being streamed
      done: bool,                   // Indicates completion
  }
  ```
- **UI Integration**: Tool call deltas can be rendered progressively like text tokens

### M3: Load-Ready

#### Back-pressure Management
- **MetricsAck Messages**: Clients send acknowledgments with `last_seq` to indicate received messages
- **Inflight Window**: Server tracks `inflight_deltas` and pauses when exceeding threshold
- **Flow Control**: Automatic pause/resume based on client acknowledgments
- **Client Metrics**: Optional client buffer state reporting

#### Persistence Strategy
- **Session Persistence**: SQLite-based session state storage
- **Message History**: Conversation context persisted across reconnections
- **Configurable Retention**: Automatic cleanup of old sessions
- **Write-through Cache**: Immediate persistence without async operations

## Protocol Flow

### 1. Session Opening

```json
// Request: chat_open
{
  "jsonrpc": "2.0",
  "method": "chat_open",
  "params": {
    "token": "eyJhbGciOiJIUzI1NiIs...",  // Optional JWT
    "context": {},                        // Optional initial context
    "metadata": {}                        // Optional metadata
  },
  "id": 1
}

// Response
{
  "jsonrpc": "2.0",
  "result": {
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "capabilities": {
      "max_context_length": 4096,
      "supported_message_types": ["text", "tool_call"],
      "supports_attachments": false,
      "supports_tools": true
    },
    "created_at": "2024-01-15T10:30:00Z"
  },
  "id": 1
}
```

### 2. Message Streaming

```json
// Subscribe to deltas
{
  "jsonrpc": "2.0",
  "method": "chat_stream",
  "params": {
    "session_id": "550e8400-e29b-41d4-a716-446655440000"
  },
  "id": 2
}

// Text delta
{
  "jsonrpc": "2.0",
  "method": "chat_stream",
  "params": {
    "subscription": "sub_123",
    "result": {
      "session_id": "550e8400-e29b-41d4-a716-446655440000",
      "message_id": "msg_456",
      "sequence": 0,
      "delta": {
        "type": "text",
        "text": "Hello, how can I "
      },
      "timestamp": "2024-01-15T10:30:01Z"
    }
  }
}

// Tool call delta (initial)
{
  "jsonrpc": "2.0",
  "method": "chat_stream",
  "params": {
    "subscription": "sub_123",
    "result": {
      "session_id": "550e8400-e29b-41d4-a716-446655440000",
      "message_id": "msg_456",
      "sequence": 5,
      "delta": {
        "type": "toolCall",
        "tool_call_id": "call_789",
        "name": "get_weather",
        "args_json_fragment": "{\"location\":\"",
        "done": false
      },
      "timestamp": "2024-01-15T10:30:02Z"
    }
  }
}

// Tool call delta (continuation)
{
  "jsonrpc": "2.0",
  "method": "chat_stream",
  "params": {
    "subscription": "sub_123",
    "result": {
      "session_id": "550e8400-e29b-41d4-a716-446655440000",
      "message_id": "msg_456",
      "sequence": 6,
      "delta": {
        "type": "toolCall",
        "tool_call_id": "call_789",
        "args_json_fragment": "New York\"}",
        "done": true
      },
      "timestamp": "2024-01-15T10:30:02Z"
    }
  }
}
```

### 3. Back-pressure Control

```json
// Client sends metrics acknowledgment
{
  "jsonrpc": "2.0",
  "method": "chat_metrics_ack",
  "params": {
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "last_seq": 50,
    "client_metrics": {
      "buffer_size": 10,
      "processing_latency_ms": 25,
      "ready_for_more": true
    }
  },
  "id": 3
}
```

## Authentication

### JWT Authentication

The protocol supports JWT-based authentication with configurable validation:

```rust
// Creating a JWT auth backend
let backend = JwtAuthBackend::new("secret")
    .with_audience("chat-api".to_string())
    .with_issuer("auth-service".to_string());
```

### Custom Authentication

Implement the `AuthBackend` trait for custom authentication:

```rust
#[async_trait]
trait AuthBackend {
    async fn validate_token(&self, token: &str) -> Result<SessionAuth>;
    async fn validate_scopes(&self, auth: &SessionAuth, required_scopes: &[String]) -> bool;
}
```

## Back-pressure Algorithm

1. **Tracking**: Server maintains `inflight_deltas` counter per session
2. **Window Check**: Before sending each delta, check if `inflight_deltas > MAX_WINDOW`
3. **Pause**: If window exceeded, pause streaming and wait for acknowledgment
4. **Resume**: On receiving `MetricsAck`, update counters and resume if paused
5. **Adaptive**: Window size can be adjusted based on client metrics

## Persistence

### Session Storage

Sessions can be persisted using the `SessionPersistence` trait:

```rust
// SQLite implementation
let persistence = SqliteSessionPersistence::new(&db_path).await?;

// Save session
persistence.save_session(&session).await?;

// Load session
let session = persistence.load_session(&session_id).await?;

// Cleanup old sessions
persistence.cleanup_old_sessions(older_than).await?;
```

### Message History

Conversation context is maintained and can be restored:

```rust
// Save message
persistence.save_message(&message).await?;

// Load conversation history
let messages = persistence.load_messages(&session_id).await?;
```

## Error Handling

### Authentication Errors
- **-32004**: Invalid or expired JWT token
- **-32005**: Insufficient scopes for operation

### Session Errors
- **-32100**: Session not found
- **-32101**: Session already exists
- **-32102**: Session closed

### Stream Errors
- **-32200**: Stream error during generation
- **-32201**: Back-pressure limit exceeded

## Configuration

### Environment Variables

- `ARKAVO_MAX_INFLIGHT_DELTAS`: Maximum unacknowledged deltas (default: 100)
- `ARKAVO_SESSION_TTL_SECONDS`: Session timeout in seconds (default: 3600)
- `ARKAVO_SESSION_DB_PATH`: Path to SQLite database for persistence
- `ARKAVO_JWT_SECRET`: Secret key for JWT validation (if using symmetric keys)

### Server Configuration

```rust
let config = ServerConfig::builder()
    .with_auth_backend(jwt_backend)
    .with_persistence(sqlite_persistence)
    .with_max_inflight_window(100)
    .with_session_ttl(3600)
    .build();
```

## Security Considerations

1. **JWT Validation**: Always validate JWT signatures and expiration
2. **Scope Checking**: Verify user scopes before executing tool calls
3. **Rate Limiting**: Apply per-session and global rate limits
4. **TLS**: Always use TLS/mTLS for transport security
5. **Session Cleanup**: Regularly clean up expired sessions

## Performance Optimization

1. **Delta Batching**: Group multiple small deltas when possible
2. **Compression**: Use WebSocket compression for large payloads
3. **Connection Pooling**: Reuse database connections for persistence
4. **Caching**: Cache frequently accessed session data in memory
5. **Async Operations**: Use async I/O for all blocking operations

## Migration from v1

### Breaking Changes
- `chat_open` now accepts optional JWT token
- Tool call deltas use new structure with `args_json_fragment`
- Back-pressure requires client acknowledgments

### Upgrade Path
1. Update client to handle new delta structure
2. Implement metrics acknowledgment sending
3. Add JWT token to `chat_open` if using authentication
4. Handle back-pressure pauses in streaming

## Examples

### Simple Chat Session
```rust
// Open session
let request = ChatOpenRequest {
    token: Some(jwt_token),
    context: None,
    metadata: None,
};
let session = client.chat_open(request).await?;

// Send message
client.chat_send(&session.session_id, message).await?;

// Subscribe to deltas
let mut stream = client.chat_stream(&session.session_id).await?;
while let Some(delta) = stream.next().await {
    process_delta(delta);
    
    // Send acknowledgment periodically
    if should_ack() {
        client.chat_metrics_ack(&session.session_id, last_seq).await?;
    }
}
```

### With Persistence
```rust
// Setup persistence
let persistence = SqliteSessionPersistence::new(&db_path).await?;

// Create manager with persistence
let manager = ChatSessionManager::with_persistence(llm_adapter, persistence);

// Sessions are automatically persisted
let session = manager.create_session(auth).await;
```

## Testing

Run integration tests with:
```bash
cargo test -p arkavo-protocol chat_protocol_v2
```

Use cargo-nextest for better test output:
```bash
cargo nextest run -p arkavo-protocol chat_protocol_v2
```