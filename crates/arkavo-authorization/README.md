# arkavo-authorization

OpenTDF Authorization v2 integration for Arkavo Edge - Connect protocol client for entitlement-based access control.

## Overview

This crate provides integration with the [OpenTDF](https://opentdf.io) platform's Authorization v2 service, enabling fine-grained access control for MCP (Model Context Protocol) tool execution. It uses the Connect protocol over HTTP/JSON for efficient communication with OpenTDF services.

## Features

- **Authorization v2 API Support**: Full implementation of GetDecision, GetDecisionMultiResource, and GetDecisionBulk methods
- **Entity Resolution v2**: Automatic JWT token resolution to entity identifiers via ERS
- **Connect Protocol**: Modern RPC over HTTP/JSON, compatible with OpenTDF's latest APIs
- **Smart Caching**: TTL-aware decision caching to minimize latency
- **Fail-Closed Security**: Denies access by default with configurable safe tool allowlist
- **Batch Operations**: Efficient bulk authorization for multiple tools
- **Zero-Config Defaults**: Works out-of-the-box with sensible defaults

## Architecture

```
JWT Token → Entity Resolution Service v2 → EntityIdentifier
                                               ↓
EntityIdentifier + Action + Resources → Authorization Service v2
                                               ↓
                                         Decision (Permit/Deny)
                                               ↓
                                         Cache → MCP Tool Access
```

## Usage

```rust
use arkavo_authorization::{AuthorizationClient, AuthorizationConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client with default config
    let config = AuthorizationConfig::default();
    let client = AuthorizationClient::new(config)?;
    
    // Authorize a single MCP tool
    let token = "eyJ..."; // JWT token
    let decision = client.authorize_mcp_tool(token, "git.commit").await?;
    
    match decision {
        Decision::Permit => println!("Tool execution allowed"),
        Decision::Deny => println!("Tool execution denied"),
    }
    
    // Authorize multiple tools at once
    let tools = vec!["git.commit", "filesystem.read", "device.tap"];
    let results = client.authorize_mcp_tools_bulk(token, tools).await?;
    
    for (tool, decision) in results {
        println!("{}: {:?}", tool, decision);
    }
    
    Ok(())
}
```

## Configuration

Configure via environment variables:

- `OPENTDF_BASE_URL`: OpenTDF platform endpoint (default: `https://platform.opentdf.io`)
- `OIDC_ISSUER`: OIDC issuer for token validation
- `AUD`: Expected audience claim in JWT tokens

Or programmatically:

```rust
let config = AuthorizationConfig::default()
    .with_base_url("https://your-opentdf-instance.com")?
    .with_timeout(Duration::from_secs(10))
    .with_cache_ttl(Duration::from_secs(120));
```

## MCP Tool Mapping

Tools are mapped to OpenTDF resource attributes using a standardized namespace:

- `filesystem.read` → `https://arkavo.net/attr/mcp-tool/value/filesystem.read`
- `git.commit` → `https://arkavo.net/attr/mcp-tool/value/git.commit`
- `device.tap` → `https://arkavo.net/attr/mcp-tool/value/device_management.tap`

Safe diagnostic tools are always permitted without authorization:
- `status`, `health`, `version`, `list_tools`

## Security Model

1. **Fail-Closed**: All tools denied by default unless explicitly permitted
2. **JWT Validation**: Tokens validated through OpenTDF's Entity Resolution Service
3. **Attribute-Based Access Control**: Fine-grained permissions based on resource attributes
4. **Caching**: Decisions cached with TTL based on token expiration (max 60s)
5. **Safe Tools**: Minimal set of diagnostic tools allowed without authorization

## API Methods

### Authorization Service v2

- `GetDecision`: Single resource authorization
- `GetDecisionMultiResource`: Multiple resources, same entity and action
- `GetDecisionBulk`: Multiple independent authorization requests

### Entity Resolution Service v2

- `CreateEntityChainsFromTokens`: Convert JWT tokens to entity identifiers

## Testing

Run tests with mock OpenTDF server:

```bash
cargo test -p arkavo-authorization
```

## License

Apache-2.0