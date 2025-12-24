# WebSocket Transport Implementation - Issue #424

## Overview
Implement WebSocket transport support alongside existing HTTP transport for Agent-to-Agent (A2A) communication, enabling real-time streaming and efficient mesh operations.

## Implementation Tasks

### 1. Core Transport Infrastructure
- [x] Review existing WebSocket implementation in `crates/arkavo-protocol/src/websocket.rs`
- [x] Review existing HTTP implementation in `crates/arkavo-protocol/src/http.rs`
- [x] Review PeerManager in `crates/arkavo-cli/src/peer_manager.rs`
- [x] Create transport selection logic based on method type
- [x] Add configuration for transport preference

### 2. PeerManager Updates
- [x] Update PeerConnection struct to support both HTTP and WebSocket transports
- [x] Implement transport selection logic (HTTP vs WebSocket)
- [x] Add method to determine if a method requires streaming
- [x] Update connect_to_peer to support WebSocket connections
- [x] Add WebSocket connection pooling/management

### 3. Transport Selection Logic
- [x] Create TransportType enum (Http, WebSocket)
- [x] Implement method classification (stateless vs stateful/streaming)
- [x] Add configuration option for default transport
- [x] Implement automatic upgrade logic for streaming methods

### 4. Integration & Testing
- [x] Update existing tests to cover both transports
- [x] Add integration tests for transport selection
- [x] Add tests for streaming methods (chat_stream, message/stream)
- [ ] Test mesh formation with WebSocket transport (requires full build environment)
- [x] Verify backward compatibility with HTTP-only peers

### 5. Documentation
- [x] Update PeerManager documentation
- [x] Add transport selection guide
- [x] Document streaming method support
- [x] Add examples for WebSocket usage

## Streaming Methods
Methods that should use WebSocket by default:
- `chat_stream`
- `message/stream`
- Any method with `_stream` suffix
- Mesh-related high-frequency operations

## Acceptance Criteria
- [x] WebSocketTransport already exists and implements A2aTransport trait
- [x] PeerManager can establish WebSocket connections
- [x] Transport selection logic intelligently chooses HTTP vs WebSocket
- [x] Streaming methods automatically use WebSocket
- [x] Configuration allows forcing specific transport
- [x] Unit tests pass with both transports
- [x] Documentation is complete and clear