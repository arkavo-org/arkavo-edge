# Secure Agent Configuration Transport

This document describes how configuration bundles are securely transported to agents using TDF (Trusted Data Format) encryption and the A2A protocol.

## Architecture Overview

```
┌─────────────────┐                    ┌─────────────────┐
│   Orchestrator  │                    │      Agent      │
│                 │                    │                 │
│ ┌─────────────┐ │     A2A Protocol   │ ┌─────────────┐ │
│ │   Config    │ │    agent.config.*  │ │   Config    │ │
│ │  Transport  │─┼───────────────────►│ │  Transport  │ │
│ │   Server    │ │                    │ │   Client    │ │
│ └─────────────┘ │                    │ └─────────────┘ │
│        │        │                    │        │        │
│        ▼        │                    │        ▼        │
│ ┌─────────────┐ │                    │ ┌─────────────┐ │
│ │    TDF      │ │                    │ │    TDF      │ │
│ │  Encryptor  │ │                    │ │  Decryptor  │ │
│ └─────────────┘ │                    │ └─────────────┘ │
└────────┬────────┘                    └────────┬────────┘
         │                                      │
         │         ┌─────────────┐              │
         └────────►│   Arkavo    │◄─────────────┘
                   │     KAS     │
                   │(Key Access) │
                   └─────────────┘
```

## Crates Involved

| Crate | Purpose |
|-------|---------|
| `arkavo-config-bundle` | Configuration bundle data structures |
| `arkavo-config-encryption` | TDF encryption/decryption with opentdf-rs |
| `arkavo-config-transport` | A2A protocol transport layer |

## Configuration Bundle

A `ConfigurationBundle` contains everything an agent needs to operate:

```rust
pub struct ConfigurationBundle {
    pub bundle_id: Uuid,
    pub target: BundleTarget,           // Agent, Role, or AttributePattern
    pub settings: HashMap<String, Value>,
    pub role: AgentRole,
    pub entitlements: Vec<Entitlement>, // Resource permissions
    pub secrets: HashMap<String, Secret>,
    pub required_attributes: Vec<String>,
    pub metadata: BundleMetadata,
}
```

**Bundle Targets:**
- `Agent(String)` - Specific agent by ID
- `Role(String)` - All agents with a role
- `AttributePattern(String)` - Agents matching attribute pattern

## TDF Encryption

Configuration bundles are encrypted using the OpenTDF format via the `opentdf-rs` crate. The encryption binds cryptographic access to policy attributes.

### Attribute FQN Format

Attributes follow the OpenTDF Fully Qualified Name (FQN) specification:

```
https://arkavo.ai/attr/{attribute}/value/{value}
```

**Examples:**
- `https://arkavo.ai/attr/data/clearance/value/confidential`
- `https://arkavo.ai/attr/capability/filesystem/value/read_content`
- `https://arkavo.ai/attr/agent/identity/value/autonomous_service`

### Arkavo Attribute Schema

From `ai_studio_code.json`, the defined attributes are:

| Attribute | Rule | Values |
|-----------|------|--------|
| `attr/model/tier` | hierarchy | low_cost, standard, advanced, reasoning |
| `attr/data/clearance` | hierarchy | public, internal, confidential, restricted, critical |
| `attr/capability/filesystem` | hierarchy | none, navigate, read_content, modify, delete |
| `attr/capability/network` | anyOf | localhost_only, intranet_access, internet_search, api_write, remote_execution |
| `attr/tools/specific` | anyOf | cmd_ls, cmd_cd, cmd_pwd, cmd_cat, cmd_grep, etc. |
| `attr/agent/identity` | anyOf | user_delegate, autonomous_service, audit_bot, developer_debug |

### Policy Definition

```rust
use arkavo_config_encryption::{Policy, PolicyAttribute, create_fqn};

let mut policy = Policy::new();
policy.add_attribute(
    "data/clearance".to_string(),
    "confidential".to_string(),
    "Data Clearance".to_string(),
);
policy.add_attribute(
    "capability/filesystem".to_string(),
    "read_content".to_string(),
    "Filesystem Capability".to_string(),
);
policy.add_dissemination("agent.role=monitoring-agent".to_string());
```

### Encryption Flow

```rust
use arkavo_config_encryption::{ConfigBundleEncryptor, Policy};

// Create encryptor with KAS URL
let encryptor = ConfigBundleEncryptor::new(
    "https://platform.arkavo.net/kas/v2/rewrap".to_string()
);

// Encrypt bundle with policy
let encrypted = encryptor.encrypt_bundle(&bundle, policy)?;
// Returns EncryptedBundle with TDF-encrypted data
```

The encryptor:
1. Validates the bundle
2. Serializes to JSON
3. Builds OpenTDF policy with attribute FQNs
4. Encrypts using `Tdf::encrypt()` from opentdf-rs
5. Returns `EncryptedBundle` with policy manifest

## Transport Envelope

The encrypted bundle is wrapped in a transport envelope for A2A transmission:

```rust
pub struct ConfigTransportEnvelope {
    pub version: String,              // "1.0.0"
    pub payload_type: String,         // "encrypted_bundle"
    pub encrypted_payload: String,    // Base64-encoded EncryptedBundle
    pub signature: String,            // Orchestrator signature
    pub timestamp: DateTime<Utc>,
    pub metadata: TransportMetadata,
}

pub struct TransportMetadata {
    pub bundle_id: String,
    pub target: String,
    pub kas_url: String,
    pub required_attributes: Vec<String>,
}
```

## Server-Side (Orchestrator)

The `ConfigTransportServer` distributes bundles:

```rust
use arkavo_config_transport::ConfigTransportServer;

let server = ConfigTransportServer::new(
    "https://platform.arkavo.net/kas/v2/rewrap".to_string()
)?;

// Distribute a bundle
let bundle_id = server.distribute_bundle(bundle, policy).await?;

// Handle agent config request
let response = server.handle_config_request(&request, agent_public_key).await?;
```

**Distribution flow:**
1. Validate bundle
2. Encrypt with TDF using policy
3. Sign encrypted bundle with orchestrator key
4. Wrap in transport envelope
5. Register in bundle registry
6. Serve via A2A `agent.config.get` RPC

## Client-Side (Agent)

The `ConfigTransportClient` receives bundles:

```rust
use arkavo_config_transport::ConfigTransportClient;
use arkavo_config_encryption::AgentIdentity;

// Create agent identity with attributes
let mut attributes = HashMap::new();
attributes.insert("agent.role".to_string(), "monitoring-agent".to_string());
attributes.insert("environment".to_string(), "production".to_string());

let identity = AgentIdentity::new("agent-001".to_string(), attributes)?;
let client = ConfigTransportClient::new(identity);

// Request and process configuration
let bundle = client.process_configuration_response(&response, &key)?;
client.apply_configuration(&bundle)?;
```

**Reception flow:**
1. Request config via A2A `agent.config.get`
2. Parse transport envelope from response
3. Extract encrypted bundle
4. Verify agent has required attributes
5. Decrypt using KAS (requires async with `decrypt_bundle_async`)
6. Apply configuration to agent

## KAS Integration

Decryption requires the Arkavo Key Access Service (KAS):

```rust
#[cfg(feature = "kas")]
async fn decrypt_with_kas(
    encrypted: &EncryptedBundle,
    identity: &AgentIdentity,
) -> Result<ConfigurationBundle> {
    let kas_client = opentdf::KasClient::new(
        "https://platform.arkavo.net/kas/v2/rewrap",
        &oauth_token
    )?;

    let decryptor = ConfigBundleDecryptor::new(identity.clone());
    decryptor.decrypt_bundle_async(encrypted, kas_client).await
}
```

**KAS Configuration:**
```rust
use arkavo_config_encryption::KasConfig;

// Production KAS
let kas = KasConfig::production();
// URL: https://platform.arkavo.net/kas/v2/rewrap

// From environment
let kas = KasConfig::from_env();
// Reads ARKAVO_KAS_URL, ARKAVO_IDENTITY_URL, ARKAVO_KAS_TOKEN

// Custom
let kas = KasConfig::custom(
    "https://custom.kas/rewrap",
    "https://custom.identity"
).with_token("oauth-token");
```

## Security Properties

| Property | Mechanism |
|----------|-----------|
| Confidentiality | AES-256-GCM encryption (TDF) |
| Integrity | HMAC in TDF, signature on envelope |
| Authenticity | Ed25519 signatures |
| Authorization | ABAC via KAS policy evaluation |
| Key Protection | Wrapped DEK, KAS-controlled access |

## Attribute-Based Access Control

Access is granted when the agent's attributes satisfy the policy:

1. Orchestrator defines policy with required attributes
2. Agent presents its attributes to KAS
3. KAS evaluates attributes against policy
4. If satisfied, KAS returns unwrapped data encryption key
5. Agent decrypts configuration

**Example policy check:**
```rust
// Policy requires: agent.role=monitoring-agent AND environment=production
// Agent has: { "agent.role": "monitoring-agent", "environment": "production" }
// Result: Access granted
```

## A2A Protocol Integration

Configuration transport uses these A2A RPC methods:

| Method | Direction | Purpose |
|--------|-----------|---------|
| `agent.config.get` | Agent → Orchestrator | Request configuration |
| `agent.config.update` | Orchestrator → Agent | Push configuration update |

The transport envelope is serialized as the `content` field in `AgentConfigGetResponse`.

## Example: Complete Flow

```rust
// === ORCHESTRATOR SIDE ===

// Create bundle for monitoring agents
let mut bundle = ConfigurationBundle::new(
    BundleTarget::Role("monitoring-agent".to_string()),
    AgentRole {
        name: "monitoring-agent".to_string(),
        purpose: "System monitoring".to_string(),
        capabilities: vec!["metrics".to_string(), "alerting".to_string()],
    },
    "orchestrator".to_string(),
    "production".to_string(),
);
bundle.add_setting("metrics_interval".to_string(), json!(60));
bundle.add_required_attribute("agent.role=monitoring-agent".to_string());

// Define access policy
let mut policy = Policy::new();
policy.add_attribute(
    "data/clearance".to_string(),
    "internal".to_string(),
    "Data Clearance".to_string(),
);
policy.add_dissemination("agent.role=monitoring-agent".to_string());

// Distribute
let server = ConfigTransportServer::new(KAS_URL.to_string())?;
let bundle_id = server.distribute_bundle(bundle, policy).await?;

// === AGENT SIDE ===

// Agent identity with matching attributes
let mut attrs = HashMap::new();
attrs.insert("agent.role".to_string(), "monitoring-agent".to_string());
let identity = AgentIdentity::new("agent-001".to_string(), attrs)?;

// Receive via A2A
let client = ConfigTransportClient::new(identity);
let config = client.process_configuration_response(&response, &key)?;

// Apply
client.apply_configuration(&config)?;
```
