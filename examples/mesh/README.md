# Mesh Networking

<!-- ARKAVO-CAPABILITY: protocol -->
> **Specs**: [19 scenarios](../../specs/arkavo-edge/protocol.spec.yaml)
> **Browse**: `cargo xtask capabilities protocol`
<!-- /ARKAVO-CAPABILITY -->

Dynamic peer-to-peer agent mesh using A2A protocol and mDNS discovery.

## Quick Start

```bash
# Start a 3-agent mesh
./mesh.sh start 3
```

## How It Works

Agents discover each other via mDNS and communicate using the A2A (Agent-to-Agent) protocol over HTTP/WebSocket. Each agent advertises its capabilities and can route tasks to peers.

## Files

| File | Purpose |
|------|---------|
| `agent-0/` | First mesh agent |
| `mesh.sh` | Launch script |

## Learn More

- [A2A Protocol Spec](../../specs/arkavo-edge/protocol.spec.yaml)
- [arkavo-protocol crate](../../crates/arkavo-protocol/)
