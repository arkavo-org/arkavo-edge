# KAS as A2A Capability

Demonstrates the Key Access Service (KAS) exposed as A2A JSON-RPC methods for TDF encryption key operations with NTDF delegation-based authorization. Peers are discovered via mDNS (`runtime.mdns: true` in the kit).

## What You'll Learn

- How to enable the KAS capability on an agent
- Using `kas.publicKey` to retrieve the KAS public key
- Using `kas.rewrap` to rewrap TDF encryption keys
- NTDF delegation token chains for access control
- ABAC (Attribute-Based Access Control) policy evaluation

## Quick Start

Build with the KAS feature enabled:

```bash
cargo build --features kas
```

Start the KAS-enabled agent:

```bash
./launch.sh
```

Test the JSON-RPC methods:

```bash
# Get KAS public key
curl -X POST http://localhost:8080 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{"algorithm":"RSA-OAEP"}}'

# Check Agent Card for KAS skills
curl http://localhost:8081/.well-known/agent.json | jq '.skills[] | select(.id | startswith("kas"))'
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         A2A JSON-RPC                             │
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │ kas.publicKey│    │  kas.rewrap  │    │ Agent Card   │       │
│  └──────┬───────┘    └──────┬───────┘    └──────────────┘       │
│         │                   │                                    │
│         ▼                   ▼                                    │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                   KasA2aHandler                          │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐      │    │
│  │  │ Delegation  │  │    ABAC     │  │  KAS        │      │    │
│  │  │  Verifier   │  │  Evaluator  │  │  Keypair    │      │    │
│  │  └─────────────┘  └─────────────┘  └─────────────┘      │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

## JSON-RPC Methods

### kas.publicKey

Retrieve the KAS public key for encrypting TDF payloads.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "kas.publicKey",
  "params": {
    "algorithm": "RSA-OAEP"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "public_key": "-----BEGIN PUBLIC KEY-----\nMIIB...\n-----END PUBLIC KEY-----",
    "key_id": "kas-key-1",
    "algorithm": "RSA-OAEP"
  }
}
```

### kas.rewrap

Rewrap a TDF encryption key for a specific client after verifying delegation and ABAC policy.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "kas.rewrap",
  "params": {
    "wrapped_key": "BASE64_WRAPPED_KEY...",
    "policy_binding": {
      "alg": "HS256",
      "hash": "BASE64_HMAC..."
    },
    "policy": "BASE64_POLICY_JSON...",
    "delegation_token": "{\"issuer_did\":\"did:key:z6Mk...\", ...}",
    "client_public_key": "-----BEGIN PUBLIC KEY-----\n..."
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "entity_wrapped_key": "BASE64_REWRAPPED_KEY..."
  }
}
```

## NTDF Delegation Tokens

Delegation tokens form a chain of trust from a root authority to the requesting agent:

```
Root Authority (Trusted)
    │
    ▼ signs token for
Intermediate Agent (delegated entitlements)
    │
    ▼ signs token for
Requesting Agent (subset of entitlements)
```

Each token contains:
- `issuer_did` - DID:key of the signing entity
- `subject_did` - DID:key of the recipient
- `entitlements` - FQN list like `["https://arkavo.net/attr/role/value/admin"]`
- `expires_at` - Expiration timestamp
- `signature` - Ed25519 signature (base64)
- `parent` - Parent token in the chain (optional)

## ABAC Policy Evaluation

TDF policies specify required attributes. The ABAC evaluator checks if the delegation token's entitlements satisfy the policy:

**TDF Policy:**
```json
{
  "attributes": [
    {
      "attribute": "https://arkavo.net/attr/role",
      "values": ["admin", "operator"]
    }
  ]
}
```

**Entitlement Required:**
```
https://arkavo.net/attr/role/value/admin
  OR
https://arkavo.net/attr/role/value/operator
```

## Files

| File | Purpose |
|------|---------|
| `kas-agent.swarmkit.yaml` | Agent config with KAS capability enabled |
| `launch.sh` | Start the KAS-enabled agent |
| `stop.sh` | Stop the agent |
| `RUNBOOK.md` | Step-by-step walkthrough |
| `test-kas.sh` | Test script for KAS methods |

## Error Codes

| Code | Meaning |
|------|---------|
| -32001 | Access denied (ABAC policy failure) |
| -32002 | Delegation verification failed |
| -32003 | Invalid policy binding |
| -32603 | KAS capability/keypair not configured |

## Next Steps

- Integrate with TDF client libraries for full encryption/decryption
- Set up trusted roots for production delegation chains
- Configure KAS keypair with real RSA keys
- See `../CONCEPTS.md` for more on A2A protocols
