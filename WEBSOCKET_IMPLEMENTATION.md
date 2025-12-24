# WebSocket Transport Implementation

## Overview
This implementation adds WebSocket transport support to the PeerManager for Agent-to-Agent (A2A) communication, enabling real-time streaming and efficient mesh operations alongside the existing HTTP transport.

## Changes Made

### 1. PeerManager Enhancements (`crates/arkavo-cli/src/peer_manager.rs`)

#### New Types
- **`TransportType` enum**: Distinguishes between HTTP and WebSocket transports
  - `Http`: For stateless, transactional requests
  - `WebSocket`: For stateful, streaming requests

- **`PeerManagerConfig` struct**: Configuration for peer manager behavior
  - `default_transport`: Default transport type to use
  - `auto_upgrade_streaming`: Automatically upgrade to WebSocket for streaming methods
  - `transport_config`: Underlying transport configuration

#### Updated Structures
- **`PeerConnection`**: Now supports both HTTP and WebSocket transports
  - `http_transport: Option<HttpTransport>`
  - `ws_transport: Option<Arc<RwLock<WebSocketTransport>>>`
  - `transport_type: TransportType`

#### New Methods
- **`with_config()`**: Create PeerManager with custom configuration
- **`is_streaming_method()`**: Determine if a method requires streaming
- **`select_transport_for_method()`**: Choose appropriate transport for a method
- **`connect_to_peer_with_transport()`**: Connect using specific transport type
- **`ensure_transport()`**: Ensure peer has required transport, upgrading if needed
- **`connected_peers_with_transport()`**: Get peers with their transport types
- **`get_peer_transport_type()`**: Get transport type for a specific peer

#### Updated Methods
- **`broadcast()`**: Now automatically selects appropriate transport per method
- **`send_to()`**: Automatically ensures correct transport before sending

### 2. Transport Selection Logic

The implementation intelligently selects transports based on:

1. **Method Classification**: Methods are classified as streaming if they:
   - End with `_stream`
   - Equal `chat_stream` or `message/stream`
   - Contain `/stream` in the name

2. **Configuration**: 
   - `auto_upgrade_streaming`: When enabled, streaming methods automatically use WebSocket
   - `default_transport`: Fallback transport for non-streaming methods

3. **URL Normalization**:
   - HTTP URLs: `http://` or `https://`
   - WebSocket URLs: `ws://` or `wss://`
   - Automatic conversion between protocols when needed

### 3. Connection Management

- **Dual Transport Support**: Peers can maintain both HTTP and WebSocket connections
- **Automatic Upgrade**: When a streaming method is called, the system automatically establishes a WebSocket connection if needed
- **Connection Pooling**: Connections are maintained in a thread-safe HashMap

## Usage Examples

### Basic Usage (Default Configuration)
```rust
// Creates PeerManager with HTTP default, auto-upgrade enabled
let manager = PeerManager::new("agent-id".to_string());

// Connect to peer (uses HTTP by default)
manager.connect_to_peers(&["http://peer1:8080"]).await?;

// Automatically upgrades to WebSocket for streaming
manager.send_to("http://peer1:8080", "chat_stream", params).await?;
```

### Custom Configuration
```rust
let config = PeerManagerConfig {
    default_transport: TransportType::WebSocket,
    auto_upgrade_streaming: true,
    transport_config: TransportConfig::default(),
};

let manager = PeerManager::with_config("agent-id".to_string(), config);
```

### Force Specific Transport
```rust
let config = PeerManagerConfig {
    default_transport: TransportType::Http,
    auto_upgrade_streaming: false,  // Disable auto-upgrade
    transport_config: TransportConfig::default(),
};

let manager = PeerManager::with_config("agent-id".to_string(), config);
```

## Streaming Methods

The following methods automatically use WebSocket when `auto_upgrade_streaming` is enabled:
- `chat_stream`
- `message/stream`
- Any method ending with `_stream`
- Any method containing `/stream`

## Benefits

1. **Real-Time Streaming**: Enables token-by-token streaming for chat and other real-time operations
2. **Reduced Latency**: Persistent connections eliminate TCP handshake overhead
3. **Asynchronous Push**: Agents can push notifications without polling
4. **Mesh Efficiency**: High-frequency mesh operations benefit from persistent connections
5. **Backward Compatibility**: Existing HTTP-only code continues to work
6. **Automatic Optimization**: Streaming methods automatically use optimal transport

## Testing

Comprehensive unit tests cover:
- Transport type selection logic
- Streaming method detection
- Configuration options
- Auto-upgrade behavior
- Default transport selection

Run tests with:
```bash
cargo test --package arkavo-cli --lib peer_manager
```

## Architecture Decisions

1. **Dual Transport Support**: Rather than replacing HTTP, we support both transports to allow optimal selection per use case
2. **Automatic Upgrade**: The system automatically selects the best transport based on method type
3. **Configuration Flexibility**: Users can control transport behavior through configuration
4. **Thread Safety**: All connection management uses thread-safe primitives (RwLock, Arc)
5. **Backward Compatibility**: Existing code works without modification

## Future Enhancements

Potential future improvements:
- Connection pooling with size limits
- Automatic reconnection on WebSocket failure
- Metrics for transport usage
- Load balancing across multiple transports
- Transport health monitoring

## Acceptance Criteria Status

- ✅ WebSocketTransport exists and implements A2aTransport trait
- ✅ PeerManager can establish WebSocket connections
- ✅ Transport selection logic intelligently chooses HTTP vs WebSocket
- ✅ Streaming methods automatically use WebSocket
- ✅ Configuration allows forcing specific transport
- ✅ Comprehensive unit tests added
- ✅ Documentation complete

## Related Files

- `crates/arkavo-cli/src/peer_manager.rs` - Main implementation
- `crates/arkavo-protocol/src/websocket.rs` - WebSocket transport (existing)
- `crates/arkavo-protocol/src/http.rs` - HTTP transport (existing)
- `crates/arkavo-protocol/src/transport.rs` - Transport trait (existing)