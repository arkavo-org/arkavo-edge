# arkavo-config-encryption

OpenTDF-based encryption and decryption for secure agent configuration distribution.

## Overview

This crate provides secure configuration bundle encryption using [OpenTDF](https://github.com/arkavo-org/opentdf-rs) with attribute-based access control (ABAC). Configuration bundles are encrypted with cryptographic policy binding and can only be decrypted by agents possessing the required attributes.

## Features

- **OpenTDF Integration**: Uses official opentdf-rs library for encryption
- **Attribute-Based Access Control**: Fine-grained access control based on agent attributes
- **Policy Binding**: Cryptographic binding of access policies to data
- **KAS Integration**: Key Access Service for centralized key management
- **Public/Private Key Authentication**: Agent identity verification with ECDSA P-256

## KAS Configuration

The crate is configured to use the Arkavo production KAS by default:

```
KAS URL: https://100.arkavo.net/kas/v2/rewrap
```

### Environment Variables

- `ARKAVO_KAS_URL`: Override the default KAS URL
- `ARKAVO_KAS_TOKEN`: OAuth token for KAS authentication

### Usage

```rust
use arkavo_config_encryption::KasConfig;

// Use production KAS (default)
let kas = KasConfig::production();

// Use custom KAS
let kas = KasConfig::custom("https://custom.kas.example.com/rewrap");

// Load from environment
let kas = KasConfig::from_env();

// With OAuth token
let kas = KasConfig::production()
    .with_token("your-oauth-token");
```

## Encryption

```rust
use arkavo_config_bundle::{ConfigurationBundle, AgentRole, BundleTarget};
use arkavo_config_encryption::{
    ConfigBundleEncryptor, Policy, PolicyAttribute, KasConfig
};

// Create configuration bundle
let mut bundle = ConfigurationBundle::new(
    BundleTarget::Role("monitoring-agent".to_string()),
    AgentRole {
        name: "monitoring-agent".to_string(),
        purpose: "System monitoring".to_string(),
        capabilities: vec!["metrics".to_string()],
    },
    "orchestrator".to_string(),
    "production".to_string(),
);

bundle.add_setting("interval".to_string(), serde_json::json!(60));
bundle.add_required_attribute("agent.role=monitoring-agent".to_string());

// Define access policy
let policy = Policy {
    attributes: vec![
        PolicyAttribute {
            attribute: "agent.role".to_string(),
            display_name: "Agent Role".to_string(),
        },
    ],
    dissemination: vec!["agent.role=monitoring-agent".to_string()],
};

// Encrypt with OpenTDF
let kas = KasConfig::production();
let encryptor = ConfigBundleEncryptor::new(kas.rewrap_url().to_string());
let encrypted = encryptor.encrypt_bundle(&bundle, policy)?;
```

## Decryption

Decryption requires async KAS integration:

```rust
use arkavo_config_encryption::{AgentIdentity, ConfigBundleDecryptor};
use std::collections::HashMap;

// Create agent identity with attributes
let mut attributes = HashMap::new();
attributes.insert("agent.role".to_string(), "monitoring-agent".to_string());

let identity = AgentIdentity::new("agent-001".to_string(), attributes)?;
let decryptor = ConfigBundleDecryptor::new(identity);

// Async decryption with KAS (requires 'kas' feature)
#[cfg(feature = "kas")]
{
    use opentdf::KasClient;

    let kas_client = KasClient::new(
        "https://100.arkavo.net/kas/v2/rewrap",
        oauth_token
    )?;

    let bundle = decryptor
        .decrypt_bundle_async(&encrypted, kas_client)
        .await?;
}
```

## Agent Identity

Agents are identified by public/private key pairs:

```rust
use arkavo_config_encryption::AgentIdentity;
use std::collections::HashMap;

let mut attributes = HashMap::new();
attributes.insert("agent.role".to_string(), "worker".to_string());
attributes.insert("environment".to_string(), "production".to_string());

// Generate new identity with key pair
let identity = AgentIdentity::new("agent-123".to_string(), attributes)?;

// Sign requests
let signature = identity.sign_request(b"request data")?;

// Check attributes
let has_access = identity.has_required_attributes(&[
    "agent.role=worker".to_string(),
    "environment=production".to_string(),
]);
```

## Policy Definition

Policies control who can decrypt configuration bundles:

```rust
use arkavo_config_encryption::{Policy, PolicyAttribute};

let mut policy = Policy::new();

// Add attributes
policy.add_attribute(
    "agent.role".to_string(),
    "Agent Role".to_string()
);

policy.add_attribute(
    "environment".to_string(),
    "Environment".to_string()
);

// Add dissemination rules
policy.add_dissemination("agent.role=monitoring-agent".to_string());
policy.add_dissemination("environment=production".to_string());
```

## Examples

Run the KAS encryption/decryption example:

```bash
cargo run --example kas_encrypt_decrypt
```

With custom KAS configuration:

```bash
export ARKAVO_KAS_URL="https://custom.kas.example.com/rewrap"
export ARKAVO_KAS_TOKEN="your-token-here"
cargo run --example kas_encrypt_decrypt
```

## Architecture

```
┌─────────────────────────────────────────┐
│         Orchestrator                     │
│                                          │
│  ┌────────────────────────────────────┐ │
│  │ ConfigBundleEncryptor              │ │
│  │  - OpenTDF PolicyBuilder           │ │
│  │  - Tdf::encrypt()                  │ │
│  └────────────────────────────────────┘ │
│                 │                        │
│                 ▼                        │
│  ┌────────────────────────────────────┐ │
│  │ EncryptedBundle                    │ │
│  │  - encrypted_data (TDF)            │ │
│  │  - policy_manifest                 │ │
│  │  - bundle_id, target, timestamps   │ │
│  └────────────────────────────────────┘ │
└─────────────────────────────────────────┘
                 │
                 │ Transport (A2A)
                 ▼
┌─────────────────────────────────────────┐
│         Agent                            │
│                                          │
│  ┌────────────────────────────────────┐ │
│  │ AgentIdentity                      │ │
│  │  - agent_id                        │ │
│  │  - attributes (ABAC)               │ │
│  │  - public/private keys             │ │
│  └────────────────────────────────────┘ │
│                 │                        │
│                 ▼                        │
│  ┌────────────────────────────────────┐ │
│  │ ConfigBundleDecryptor              │ │
│  │  - Verify attributes               │ │
│  │  - Tdf::decrypt() with KAS         │ │
│  └────────────────────────────────────┘ │
│                 │                        │
│                 ▼                        │
│  ┌────────────────────────────────────┐ │
│  │ ConfigurationBundle                │ │
│  │  - settings, roles, entitlements   │ │
│  └────────────────────────────────────┘ │
└─────────────────────────────────────────┘
                 ▲
                 │
                 │ KAS rewrap
                 │
┌─────────────────────────────────────────┐
│  Arkavo KAS                              │
│  https://100.arkavo.net/kas/v2/rewrap   │
│                                          │
│  - Policy evaluation                    │
│  - Attribute verification                │
│  - Key unwrapping                        │
│  - Audit logging                         │
└─────────────────────────────────────────┘
```

## Security

- **End-to-End Encryption**: AES-256-GCM via OpenTDF
- **Policy Binding**: HMAC-SHA256 binds policies to encrypted data
- **ECDSA Signatures**: P-256 curve for request signing and verification
- **Attribute Verification**: Access denied if agent lacks required attributes
- **KAS Authorization**: Centralized key management with audit trail

## Testing

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_encrypt_bundle
```

## Features

- `default`: Basic encryption/decryption without KAS
- `kas`: Enable async KAS integration for production decryption

## Dependencies

- **opentdf**: OpenTDF Rust implementation
- **ring**: Cryptographic primitives (ECDSA)
- **serde**: Serialization
- **uuid**: Unique identifiers
- **chrono**: Timestamps

## License

Apache-2.0

## Related Crates

- `arkavo-config-bundle`: Configuration bundle data structures
- `arkavo-config-transport`: A2A transport layer
- `arkavo-protocol`: Agent-to-Agent protocol
