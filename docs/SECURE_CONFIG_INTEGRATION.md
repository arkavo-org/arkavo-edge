# Secure Configuration Distribution - A2A Integration Guide

## Overview

This guide explains how to integrate the secure configuration distribution system with the existing A2A (Agent-to-Agent) protocol infrastructure in arkavo-edge.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    ORCHESTRATOR (A2A Server)                     │
│                                                                  │
│  A2aRpcImpl                                                      │
│  ├─ ConfigTransportHandler (optional)                           │
│  │  ├─ AgentKeyRegistry (public keys)                           │
│  │  └─ handle_config_request()                                  │
│  │                                                               │
│  └─ agent_config_get() implementation                           │
│     ├─ Try secure config transport first                        │
│     └─ Fall back to AGENTS.md if not available                  │
│                                                                  │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             │ A2A Protocol (JSON-RPC)
                             │ agent.config.get
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                        AGENT (A2A Client)                        │
│                                                                  │
│  A2A Client                                                      │
│  ├─ Call agent.config.get                                       │
│  └─ Receive AgentConfigGetResponse                              │
│                                                                  │
│  ConfigTransportClient                                           │
│  ├─ process_configuration_response()                            │
│  ├─ Verify orchestrator signature                               │
│  ├─ Decrypt bundle                                              │
│  └─ apply_configuration()                                       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Components

### 1. Configuration Crates

- **arkavo-config-bundle**: Configuration bundle data structures
- **arkavo-config-encryption**: Encryption, signing, and key management
- **arkavo-config-transport**: A2A protocol transport layer

### 2. A2A Protocol Integration

- **arkavo-protocol/config_transport**: Integration module for A2A server
  - `ConfigTransportHandler`: Handles secure config requests
  - `AgentKeyRegistry`: Stores agent public keys
  - `ConfigTransportIntegration`: Integration helpers

## Integration Steps

### Step 1: Add ConfigTransportHandler to A2aRpcImpl

```rust
// In crates/arkavo-protocol/src/server.rs

use crate::config_transport::ConfigTransportHandler;

pub struct A2aRpcImpl {
    // ... existing fields ...
    
    /// Optional secure config transport handler
    config_transport: Option<Arc<ConfigTransportHandler>>,
}

impl A2aRpcImpl {
    pub fn new(/* ... existing params ... */) -> Self {
        // Initialize config transport if KAS URL is provided
        let config_transport = std::env::var("KAS_URL")
            .ok()
            .map(|kas_url| Arc::new(ConfigTransportHandler::new(kas_url)));
        
        Self {
            // ... existing fields ...
            config_transport,
        }
    }
}
```

### Step 2: Modify agent_config_get Implementation

```rust
// In crates/arkavo-protocol/src/server.rs

#[async_trait]
impl A2aRpcServer for A2aRpcImpl {
    async fn agent_config_get(
        &self,
        request: AgentConfigGetRequest,
    ) -> RpcResult<AgentConfigGetResponse> {
        let timer = RpcTimer::new("agent_config_get".to_string(), self.metrics.clone());

        // Check rate limit
        if let Err(e) = self.rate_limiter.check_rate_limit() {
            self.metrics.record_rate_limit_blocked(None);
            timer.error();
            return Err(e);
        }

        // Try secure config transport first (if available)
        if let Some(config_transport) = &self.config_transport {
            if let Ok(Some(response)) = config_transport.handle_config_request(&request).await {
                info!(
                    agent_id = %request.agent_id,
                    "Served secure configuration bundle"
                );
                timer.success();
                return Ok(response);
            }
        }

        // Fall back to standard AGENTS.md config
        info!(
            agent_id = %request.agent_id,
            "Falling back to standard AGENTS.md configuration"
        );

        // ... existing AGENTS.md reading logic ...
    }
}
```

### Step 3: Register Agent Public Keys

When agents connect or register, store their public keys:

```rust
// During agent discovery or registration

if let Some(config_transport) = &self.config_transport {
    config_transport
        .register_agent_key(agent_id.clone(), agent_public_key)
        .await;
}
```

### Step 4: Agent-Side Integration

Agents use `ConfigTransportClient` to process responses:

```rust
use arkavo_config_transport::ConfigTransportClient;
use arkavo_config_encryption::AgentIdentity;

// Create agent identity
let mut attributes = HashMap::new();
attributes.insert("agent.role".to_string(), "search-agent".to_string());
let identity = AgentIdentity::new(agent_id, attributes)?;

// Create transport client
let client = ConfigTransportClient::new(identity);

// Call A2A RPC to get config
let response = a2a_client.agent_config_get(request).await?;

// Process response (checks if it's a secure bundle)
if let Ok(bundle) = client.process_configuration_response(&response, decryption_key) {
    // Apply secure configuration
    client.apply_configuration(&bundle)?;
} else {
    // Fall back to parsing AGENTS.md format
    // ... existing config parsing ...
}
```

## Configuration

### Environment Variables

- **`KAS_URL`**: URL of the Key Access Service (e.g., `https://kas.example.com`)
  - If set, enables secure config transport
  - If not set, uses standard AGENTS.md only

### Orchestrator Setup

```bash
# Enable secure config transport
export KAS_URL=https://kas.example.com

# Start orchestrator
arkavo server
```

### Agent Setup

```bash
# Agents automatically detect secure config support
# No additional configuration needed
arkavo agent run --id my-agent
```

## Migration Path

### Phase 1: Gradual Rollout (Current)

1. Deploy orchestrator with `ConfigTransportHandler` (optional)
2. Agents continue using AGENTS.md
3. No breaking changes

### Phase 2: Hybrid Mode

1. Some agents register public keys
2. Those agents receive secure bundles
3. Other agents continue with AGENTS.md
4. Both modes work simultaneously

### Phase 3: Full Secure Mode

1. All agents register public keys
2. All agents receive secure bundles
3. AGENTS.md becomes backup/fallback only

## Security Features

### Multi-Layer Security

1. **Transport Layer**
   - HTTPS for network transport
   - Orchestrator signature verification
   - Agent request signing

2. **Encryption Layer**
   - AES-256-GCM encryption
   - Random nonces per encryption
   - Authenticated encryption

3. **Access Control Layer**
   - Attribute-based policies
   - Pre-decryption attribute validation
   - KAS authorization

### Key Management

- **Orchestrator**: ECDSA P-256 key pair for signing bundles
- **Agents**: ECDSA P-256 key pairs for authentication
- **KAS**: Centralized key management and policy enforcement

## Testing

### Unit Tests

```bash
# Test config transport integration
cargo test -p arkavo-protocol config_transport

# Test all config crates
cargo test -p arkavo-config-bundle
cargo test -p arkavo-config-encryption
cargo test -p arkavo-config-transport
```

### Integration Test

```bash
# Run the example
cargo run --example secure-config-distribution
```

### Manual Testing

1. Start orchestrator with KAS_URL set
2. Create and distribute a config bundle
3. Start agent and verify it receives the bundle
4. Check logs for "Served secure configuration bundle"

## Troubleshooting

### Agent Not Receiving Secure Config

**Symptom**: Agent falls back to AGENTS.md

**Possible Causes**:
1. Agent public key not registered
2. KAS_URL not set on orchestrator
3. Agent attributes don't match bundle requirements

**Solution**:
```bash
# Check orchestrator logs
grep "Agent public key not registered" logs

# Verify KAS_URL is set
echo $KAS_URL

# Check agent attributes match bundle requirements
```

### Decryption Failures

**Symptom**: "Decryption error" in logs

**Possible Causes**:
1. Wrong decryption key
2. Corrupted bundle
3. Signature verification failed

**Solution**:
```bash
# Check orchestrator signature
# Verify agent has correct KAS access
# Check bundle integrity
```

## Example: Complete Flow

See `examples/secure-config-distribution.rs` for a complete working example.

## API Reference

### ConfigTransportHandler

```rust
pub struct ConfigTransportHandler {
    key_registry: AgentKeyRegistry,
    kas_url: String,
}

impl ConfigTransportHandler {
    pub fn new(kas_url: String) -> Self;
    pub async fn register_agent_key(&self, agent_id: String, public_key: Vec<u8>);
    pub async fn handle_config_request(&self, request: &AgentConfigGetRequest) 
        -> Result<Option<AgentConfigGetResponse>>;
}
```

### ConfigTransportClient

```rust
pub struct ConfigTransportClient {
    agent_identity: AgentIdentity,
    decryptor: ConfigBundleDecryptor,
}

impl ConfigTransportClient {
    pub fn new(agent_identity: AgentIdentity) -> Self;
    pub fn process_configuration_response(
        &self,
        response: &AgentConfigGetResponse,
        decryption_key: &[u8],
    ) -> Result<ConfigurationBundle>;
    pub fn apply_configuration(&self, bundle: &ConfigurationBundle) -> Result<()>;
}
```

## Future Enhancements

1. **Automatic Key Distribution**: Agents automatically register public keys during discovery
2. **Bundle Versioning**: Track and manage configuration versions
3. **Rollback Support**: Revert to previous configurations
4. **Batch Distribution**: Distribute to multiple agents simultaneously
5. **Metrics Collection**: Track distribution success rates and latency
6. **CLI Commands**: User-facing commands for bundle management

## Support

For issues or questions:
- GitHub Issues: https://github.com/arkavo-org/arkavo-edge/issues
- Documentation: See crate READMEs in `crates/arkavo-config-*/`

## License

Apache-2.0