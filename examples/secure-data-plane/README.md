# Secure Data Plane: TDF + KAS + Iroh P2P

Demonstrates encrypted interagent data sharing using TDF (Trusted Data Format)
with KAS (Key Access Service) for key management and Iroh for P2P blob transport.

## What You'll Learn

- End-to-end encrypted data sharing between agents
- Iroh P2P data plane for direct agent-to-agent blob transfer
- TDF encryption with AES-256-GCM and attribute-based access control
- KAS key rewrap with NTDF delegation tokens
- Separation of control plane (A2A JSON-RPC) from data plane (Iroh)

## Architecture

```
Control Plane (A2A JSON-RPC over HTTP)
├── kas.publicKey   - Exchange encryption keys
├── tdf.share       - Send Iroh ticket + metadata
└── tdf.offers      - List pending share offers

Data Plane (Iroh P2P)
├── stage           - Add encrypted blob to local Iroh node
└── fetch           - Download encrypted blob from peer's Iroh node
```

```
┌──────────────────────────────────────────────────────────┐
│                    Data Sender                            │
│                                                           │
│  1. Read sensitive file                                   │
│  2. Get receiver's KAS public key (kas.publicKey)        │
│  3. Encrypt with TDF (AES-256-GCM + ABAC policy)        │
│  4. Stage encrypted blob to Iroh node                    │
│  5. Send ticket via A2A (tdf.share)                      │
└──────────────────┬───────────────────────────────────────┘
                   │
           Iroh P2P │ (encrypted blob)
         A2A JSON  │ (ticket + metadata)
                   │
┌──────────────────▼───────────────────────────────────────┐
│                    Data Receiver                          │
│                                                           │
│  1. Receive tdf.share offer                              │
│  2. List offers (tdf.offers)                             │
│  3. Fetch encrypted blob from Iroh                       │
│  4. Verify delegation + ABAC policy (kas.rewrap)         │
│  5. Decrypt payload                                      │
└──────────────────────────────────────────────────────────┘
```

## Quick Start

Build with KAS and Iroh features:

```bash
cargo build --features kas,iroh
```

Launch both agents:

```bash
./launch.sh
```

Run the test script:

```bash
./test-data-plane.sh
```

## Manual Walkthrough

### Step 1: Get receiver's public key

```bash
curl -s -X POST http://localhost:8082 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{"request":{}}}' | jq .
```

### Step 2: Encrypt and stage a file

```bash
# Encrypt
arkavo tdf encrypt -i secret.txt --kas-url http://localhost:8080

# Stage to Iroh
arkavo tdf stage -i secret.txt.tdf.json
```

### Step 3: Share ticket with receiver

```bash
curl -s -X POST http://localhost:8082 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0","id":2,"method":"tdf.share",
    "params":{"request":{
      "ticket":"<IROH_TICKET>",
      "content_hash":"<HASH>",
      "size_bytes":4096,
      "policy_attributes":["https://arkavo.net/attr/sensitivity"],
      "kas_url":"http://localhost:8080",
      "sender_agent_id":"data-sender"
    }}
  }' | jq .
```

### Step 4: Check offers on receiver

```bash
curl -s -X POST http://localhost:8082 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tdf.offers","params":{"request":{}}}' | jq .
```

## Agents

| Agent | Port (RPC) | Port (HTTP) | Role |
|-------|-----------|-------------|------|
| data-sender | 8080 | 8081 | Encrypts + stages data |
| data-receiver | 8082 | 8083 | Receives + decrypts data |

## Files

| File | Purpose |
|------|---------|
| `sender/AGENTS.md` | Sender agent config with KAS |
| `receiver/AGENTS.md` | Receiver agent config with KAS + trusted roots |
| `launch.sh` | Start both agents |
| `stop.sh` | Stop both agents |
| `test-data-plane.sh` | End-to-end test script |

## Security Model

- **Encryption**: AES-256-GCM via OpenTDF
- **Key Management**: EC P-256 ECDH with HKDF key derivation
- **Access Control**: ABAC (Attribute-Based Access Control)
- **Authorization**: NTDF delegation token chains
- **Transport**: Iroh QUIC with relay for NAT traversal
- **TLS**: rustls (no OpenSSL)
