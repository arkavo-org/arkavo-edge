---
name: kas-agent
purpose: "Demonstrates KAS (Key Access Service) as an A2A JSON-RPC capability"
model: ministral-3b

# KAS Capability Configuration
#
# The KAS capability enables TDF key rewrap operations via A2A JSON-RPC.
# Agents can use kas.publicKey and kas.rewrap methods for secure data sharing.
#
# Methods:
#   - kas.publicKey: Get KAS public key for TDF encryption
#   - kas.rewrap: Rewrap TDF keys with delegation verification and ABAC

kas:
  enabled: true

  # Key configuration
  key_id: "kas-demo-key-1"
  algorithm: "RSA-OAEP"

  # Trusted root authorities for delegation chains
  # These DIDs are trusted to issue delegation tokens
  trusted_roots:
    - did: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
      name: "Demo Root Authority"

# A2A Protocol Configuration
a2a:
  enabled: true
  discovery:
    mdns: true
---

# KAS Agent

This agent provides KAS (Key Access Service) capabilities via A2A JSON-RPC,
enabling secure TDF key operations with delegation-based authorization.

## Capabilities

- **kas.publicKey** - Retrieve the KAS public key for TDF encryption
- **kas.rewrap** - Rewrap TDF encryption keys with ABAC policy enforcement

## How It Works

When a client calls `kas.rewrap`:

1. **Delegation Verification** - The NTDF delegation token chain is verified:
   - Token signatures are checked using Ed25519/DID:key
   - Expiration is validated
   - Chain must terminate at a trusted root

2. **ABAC Evaluation** - Entitlements from the delegation are matched against TDF policy:
   - Policy attributes specify required access
   - Entitlements must satisfy all policy attributes
   - Decision: PERMIT or DENY

3. **Key Rewrap** - If authorized, the wrapped key is:
   - Decrypted using KAS private key
   - Re-encrypted for the client's public key
   - Returned in the response

## Usage

Start the agent:

```bash
arkavo agent run
```

Test KAS methods:

```bash
# Get public key
curl -X POST http://localhost:8080 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{}}'
```

## Agent Card

When KAS is enabled, the agent advertises these skills in `/.well-known/agent.json`:

```json
{
  "skills": [
    {
      "id": "kas.rewrap",
      "name": "TDF Key Rewrap",
      "description": "Rewrap TDF encryption keys with ABAC policy enforcement",
      "tags": ["kas", "tdf", "encryption"]
    },
    {
      "id": "kas.publicKey",
      "name": "KAS Public Key",
      "description": "Get KAS public key for TDF encryption",
      "tags": ["kas", "crypto"]
    }
  ]
}
```
