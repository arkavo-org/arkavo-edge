# arkavo-config-transport

Configuration transport layer using A2A protocol for secure configuration bundle distribution.

## Overview

This crate provides secure configuration bundle transport over the existing A2A (Agent-to-Agent) protocol infrastructure. It leverages the `agent.config.*` RPC methods to distribute encrypted configuration bundles from orchestrators to agents.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    ORCHESTRATOR                                  │
│                                                                  │
│  ConfigTransportServer                                           │
│  ├─ Encrypt bundle (AES-256-GCM)                                │
│  ├─ Sign with orchestrator private key                          │
│  ├─ Wrap in ConfigTransportEnvelope                             │
│  └─ Register in BundleRegistry                                  │
│                                                                  │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             │ A2A Protocol
                             │ (agent.config.get RPC)
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                        AGENT                                     │
│                                                                  │
│  ConfigTransportClient                                           │
│  ├─ Request config via A2A                                      │
│  ├─ Verify orchestrator signature                               │
│  ├─ Decrypt bundle with agent identity                          │
│  └─ Apply configuration                                         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Features

- **A2A Protocol Integration**: Uses existing `agent.config.get` and `agent.config.update` RPC methods
- **Secure Transport**: Encrypted bundles with signature verification
- **Public/Private Key Authentication**: ECDSA P-256 signatures for authenticity
- **Bundle Registry**: Server-side tracking of distributed bundles
- **Attribute-Based Access**: Policy enforcement before decryption

## Usage

### Server-Side (Orchestrator)

```rust
use arkavo_config_transport::ConfigTransportServer;
use arkavo_config_bundle::{ConfigurationBundle, AgentRole, BundleTarget};
use arkavo_config_encryption::Policy;

// Create transport server
let server = ConfigTransportServer::new("https://kas.example.com".to_string())?;

// Create configuration bundle
let mut bundle = ConfigurationBundle::new(
    BundleTarget::Role("search-agent".to_string()),
    AgentRole {
        name: "search-agent".to_string(),
        purpose: "Web search and data retrieval".to_string(),
        capabilities: vec!["web_search".to_string()],
    },
    "admin".to_string(),
    "production".to_string(),
);

bundle.add_required_attribute("agent.role=search-agent".to_string());
bundle.add_setting("timeout".to_string(), serde_json::json!(30));

// Create access policy
let mut policy = Policy::new();
policy.add_attribute("agent.role".to_string(), "Agent Role".to_string());
policy.add_dissemination("agent.role=search-agent".to_string());

// Distribute bundle
let bundle_id = server.distribute_bundle(bundle, policy).await?;

// Handle agent request (called by A2A RPC handler)
let response = server.handle_config_request(&request, agent_public_key).await?;
```

### Client-Side (Agent)

```rust
use arkavo_config_transport::ConfigTransportClient;
use arkavo_config_encryption::AgentIdentity;
use std::collections::HashMap;

// Create agent identity
let mut attributes = HashMap::new();
attributes.insert("agent.role".to_string(), "search-agent".to_string());
let identity = AgentIdentity::new("search-agent-001".to_string(), attributes)?;

// Create transport client
let client = ConfigTransportClient::new(identity);

// Process configuration response from A2A
let bundle = client.process_configuration_response(&response, decryption_key)?;

// Apply configuration
client.apply_configuration(&bundle)?;

// Acknowledge receipt
client.acknowledge_configuration(&bundle.bundle_id.to_string()).await?;
```

## Integration with A2A Protocol

### Existing A2A Methods Used

The transport layer leverages these existing A2A RPC methods:

1. **`agent.config.get`** - Agent requests configuration
   - Request: `AgentConfigGetRequest { agent_id, include_backups }`
   - Response: `AgentConfigGetResponse { content, version, backups, writable }`

2. **`agent.config.update`** - Orchestrator updates configuration
   - Request: `AgentConfigUpdateRequest { agent_id, content, expected_version, create_backup }`
   - Response: `AgentConfigUpdateResponse { success, new_version, backup_id }`

### Transport Envelope Format

Configuration bundles are wrapped in a `ConfigTransportEnvelope`:

```json
{
  "version": "1.0.0",
  "payload_type": "encrypted_bundle",
  "encrypted_payload": "<base64-encoded-encrypted-bundle>",
  "signature": "<base64-encoded-signature>",
  "timestamp": "2025-01-15T10:30:00Z",
  "metadata": {
    "bundle_id": "550e8400-e29b-41d4-a716-446655440000",
    "target": "Role(&quot;search-agent&quot;)",
    "kas_url": "https://kas.example.com",
    "required_attributes": ["agent.role=search-agent"]
  }
}
```

This envelope is serialized to JSON and placed in the `content` field of `AgentConfigGetResponse`.

## Security

### Encryption
- **Algorithm**: AES-256-GCM (via `arkavo-config-encryption`)
- **Key Management**: Centralized KAS (Key Access Service)
- **Nonce**: Random 96-bit nonces per encryption

### Authentication
- **Orchestrator Signature**: ECDSA P-256 signature on encrypted bundle
- **Agent Verification**: Public key verification before decryption
- **Request Signing**: Agent signs requests with private key

### Access Control
- **Attribute-Based**: Policy enforcement based on agent attributes
- **Pre-Decryption Check**: Attributes validated before decryption
- **KAS Authorization**: Additional authorization at KAS level

## Testing

Run tests with:

```bash
cargo test -p arkavo-config-transport
```

## Dependencies

- `arkavo-config-bundle` - Configuration bundle data structures
- `arkavo-config-encryption` - Encryption and key management
- `arkavo-protocol` - A2A protocol types and RPC definitions

## Future Enhancements

1. **Full A2A Client Integration**: Direct RPC calls instead of manual envelope handling
2. **Automatic Key Distribution**: Agent public key registration during discovery
3. **Bundle Versioning**: Track and manage bundle versions
4. **Rollback Support**: Revert to previous configuration versions
5. **Batch Distribution**: Distribute to multiple agents simultaneously
6. **Metrics Collection**: Track distribution success rates and latency

## License

Apache-2.0