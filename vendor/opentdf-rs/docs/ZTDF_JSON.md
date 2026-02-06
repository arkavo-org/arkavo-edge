# ZTDF-JSON: TDF for JSON-RPC Protocols

## Overview

ZTDF-JSON is an adaptation of the OpenTDF TDF3 format optimized for JSON-RPC protocols like A2A (Agent-to-Agent) and MCP (Model Context Protocol). It provides the same security guarantees as standard TDF while eliminating ZIP overhead by inlining the encrypted payload.

## Key Features

- **Inline Payload**: Encrypted data embedded directly in JSON manifest
- **JSON-Native**: Fits naturally in JSON-RPC message envelopes
- **Full OpenTDF Compatibility**: Maintains all security properties of TDF3
- **Zero Trust**: Data remains encrypted until authorized by KAS
- **ABAC Support**: Fine-grained attribute-based access control
- **Protocol Agnostic**: Works with any JSON-RPC 2.0 protocol

## Format Comparison

### Traditional TDF3 (ZIP Archive)

```
TDF Archive (.tdf)
├── manifest.json          # Encryption metadata
└── 0.payload              # Encrypted content (separate file)
```

### ZTDF-JSON (Inline)

```json
{
  "manifest": {
    "encryptionInformation": { ... },
    "payload": {
      "type": "inline",
      "value": "base64_encrypted_data"  // Inline!
    }
  },
  "version": "3.0.0"
}
```

## Usage

### Basic Encryption

```rust
use opentdf::{jsonrpc::TdfJsonRpc, Policy};

// Create policy
let policy = Policy::new(
    uuid::Uuid::new_v4().to_string(),
    vec![],
    vec!["user@example.com".to_string()]
);

// Encrypt data
let envelope = TdfJsonRpc::encrypt(b"Sensitive data")
    .kas_url("https://kas.example.com")
    .policy(policy)
    .mime_type("text/plain")
    .build()?;

// Serialize for transmission
let json = serde_json::to_string(&envelope)?;
```

### With Attribute-Based Access Control

```rust
use opentdf::{
    jsonrpc::TdfJsonRpc, 
    Policy, 
    AttributePolicy, 
    AttributeIdentifier,
    Operator
};

// Create policy with attribute requirements
let clearance = AttributePolicy::condition(
    AttributeIdentifier::from_string("gov.example:clearance")?,
    Operator::MinimumOf,
    Some("SECRET".into())
);

let policy = Policy::new(
    uuid::Uuid::new_v4().to_string(),
    vec![clearance],
    vec!["user@example.com".to_string()]
);

// Encrypt with ABAC policy
let envelope = TdfJsonRpc::encrypt(b"Classified information")
    .kas_url("https://kas.example.com")
    .policy(policy)
    .build()?;
```

### Decryption with KAS

```rust
use opentdf::{jsonrpc::TdfJsonRpc, kas::KasClient};

// Deserialize received envelope
let envelope: TdfJsonRpc = serde_json::from_str(&json)?;

// Create KAS client
let kas_client = KasClient::new(
    "https://kas.example.com",
    "oauth-token"
)?;

// Get payload key from KAS (with policy validation)
let manifest = envelope.to_standard_manifest();
let payload_key = kas_client.rewrap_standard_tdf(&manifest).await?;

// Decrypt payload
let plaintext = envelope.decrypt_with_key(&payload_key)?;
```

## Integration with JSON-RPC Protocols

### A2A (Agent-to-Agent) Protocol

```json
{
  "jsonrpc": "2.0",
  "id": 123,
  "result": {
    "message": {
      "role": "agent",
      "parts": [...],
      "tdf": {
        "manifest": { ... },
        "version": "3.0.0"
      }
    }
  }
}
```

### MCP (Model Context Protocol)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      { "type": "text", "text": "Tool executed" }
    ],
    "tdf": {
      "manifest": { ... },
      "version": "3.0.0"
    }
  }
}
```

## Security Properties

### Cryptographic Algorithms

- **Symmetric Encryption**: AES-256-GCM
- **Policy Binding**: HMAC-SHA256
- **Key Wrapping**: RSA-2048 with OAEP
- **Key Agreement**: ECDH P-256 + HKDF-SHA256

### Security Guarantees

1. **End-to-End Encryption**: Data encrypted throughout lifecycle
2. **Policy Binding**: Access policies cryptographically bound to data
3. **Zero Trust**: Keys never stored with encrypted data
4. **Integrity**: HMAC ensures policy cannot be tampered
5. **Audit Trail**: All KAS access logged for compliance

### Threat Mitigation

| Threat | Mitigation |
|--------|-----------|
| Man-in-the-Middle | mTLS + TDF encryption |
| Unauthorized Access | KAS policy enforcement |
| Policy Tampering | HMAC-SHA256 binding |
| Key Compromise | Key splitting, rotation |
| Replay Attacks | Nonce in KAS protocol |

## Protocol Extensions

### A2A Extension

Add `tdf` field to `Message` and `Artifact` objects:

```typescript
interface Message {
  role: "user" | "agent";
  parts: Part[];
  tdf?: TdfEnvelope;  // NEW
  // ... other fields
}
```

### MCP Extension

Add `tdf` field to tool response:

```typescript
interface CallToolResult {
  content: Content[];
  tdf?: TdfEnvelope;  // NEW
  // ... other fields
}
```

## Performance Considerations

### Overhead

- **Encryption**: ~5-10% overhead vs plaintext
- **JSON Size**: ~33% increase due to base64 encoding
- **KAS Latency**: 50-200ms for key unwrapping

### Optimization Tips

1. **Cache KAS Public Keys**: Reduce key fetch latency
2. **Batch Operations**: Encrypt multiple items together
3. **Streaming**: Use for large payloads (>1MB)
4. **Connection Pooling**: Reuse KAS connections

## Best Practices

### Policy Design

1. **Least Privilege**: Grant minimum required attributes
2. **Time Bounds**: Set validity periods for sensitive data
3. **Hierarchical Attributes**: Use for organizational structure
4. **Audit Logging**: Enable comprehensive access logs

### Key Management

1. **Key Rotation**: Rotate keys periodically
2. **Multi-KAS**: Use multiple KAS for redundancy
3. **Secure Storage**: Never log or store unwrapped keys
4. **Access Control**: Restrict KAS access to authorized services

### Production Deployment

1. **HTTPS Only**: Always use TLS for KAS communication
2. **Certificate Validation**: Verify KAS certificates
3. **Token Management**: Use short-lived OAuth tokens
4. **Network Isolation**: Deploy KAS in protected network

## Examples

See `examples/jsonrpc_example.rs` for a complete working example.

## References

- [OpenTDF Specification](https://github.com/opentdf/spec)
- [A2A Protocol](https://a2a-protocol.org/)
- [MCP Protocol](https://modelcontextprotocol.io/)
- [GitHub Issue #21](https://github.com/arkavo-org/opentdf-rs/issues/21)

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

This project is licensed under the same license as opentdf-rs.