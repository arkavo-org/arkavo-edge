# arkavo-protocol

Protocol adapters for the Arkavo agentic CLI tool - MCP & A2A client implementations with full mTLS support.

## Features

- JSON-RPC 2.0 client/server implementation
- HTTP and WebSocket transports
- Full mutual TLS (mTLS) support
- Rate limiting and retry logic
- Event streaming support
- OpenRPC schema generation

## mTLS Support

Both HTTP and WebSocket transports now support mutual TLS authentication for secure agent-to-agent communication.

### Configuration

```rust
use arkavo_protocol::transport::{TransportConfig, TlsConfig};
use arkavo_protocol::http::HttpTransport;

let config = TransportConfig {
    tls_config: TlsConfig {
        verify_cert: true,              // Verify server certificates
        require_tls: true,              // Require TLS (reject plain HTTP/WS)
        client_cert_path: Some("/path/to/client.crt".to_string()),
        client_key_path: Some("/path/to/client.key".to_string()),
        ca_cert_path: Some("/path/to/ca.crt".to_string()),
    },
    ..Default::default()
};

let transport = HttpTransport::new(config)?;
```

### Certificate Generation

For testing and development, you can generate self-signed certificates using the provided script:

```bash
cd crates/arkavo-protocol
./scripts/generate_test_certs.sh
```

This will create test certificates in `tests/certs/`:
- `ca.crt`, `ca.key` - Certificate Authority
- `server.crt`, `server.key` - Server certificate
- `client.crt`, `client.key` - Client certificate for mTLS

### Production Usage

For production environments:

1. **Use certificates from a trusted CA** - Replace self-signed certificates with ones from a trusted Certificate Authority
2. **Enable certificate verification** - Set `verify_cert: true` in production
3. **Require TLS** - Set `require_tls: true` to reject unencrypted connections
4. **Secure key storage** - Store private keys securely with appropriate file permissions

### Transport Support

- **HTTP Transport**: Full mTLS support using `reqwest` with `rustls`
  - Client certificate authentication
  - Custom CA certificates
  - Certificate verification control

- **WebSocket Transport**: Full mTLS support using `tokio-tungstenite` with `rustls`
  - Client certificate authentication
  - Custom CA certificates
  - Certificate verification control

### Development Mode

For development with self-signed certificates:

```rust
let config = TransportConfig {
    tls_config: TlsConfig {
        verify_cert: false,  // Accept self-signed certificates
        require_tls: false,  // Allow plain HTTP/WS for local testing
        // ... certificate paths
    },
    ..Default::default()
};
```

## Testing

Run the mTLS integration tests:

```bash
cargo test -p arkavo-protocol --test mtls_integration
```

## Security Considerations

- Always use TLS in production (`require_tls: true`)
- Verify certificates in production (`verify_cert: true`)
- Rotate certificates regularly
- Store private keys securely with restricted permissions (e.g., 0600)
- Use strong key sizes (RSA 2048+ or ECDSA P-256+)
- Monitor certificate expiration dates

## Dependencies

The mTLS implementation uses pure Rust libraries:
- `rustls` - TLS implementation
- `rustls-pemfile` - PEM file parsing
- `webpki-roots` - Mozilla's root certificates
- `rcgen` - Certificate generation (for testing only)

No OpenSSL dependency is required.