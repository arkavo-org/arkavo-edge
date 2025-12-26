# arkavo-authorization

OpenTDF Authorization v2 integration for Arkavo Edge - Connect protocol client for entitlement-based access control.

## Features

- **Authorization v2 Support**: Full integration with OpenTDF's latest Authorization service APIs.
- **Entity Resolution**: Automatic resolution of JWT tokens to entity identifiers via the Entity Resolution Service (ERS).
- **Connect Protocol Client**: Modern, efficient RPC over HTTP/JSON for high-performance authorization checks.
- **Intelligent Caching**: TTL-aware decision caching to minimize latency in tool execution.
- **Fail-Closed Security**: Default-deny security model for sensitive MCP tool execution.
- **Batch Operations**: Support for efficient bulk authorization of multiple resources or tools in a single request.
- **Attribute-Based Access Control (ABAC)**: Fine-grained permissions based on standardized resource attributes.
