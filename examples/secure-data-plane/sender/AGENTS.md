---
name: data-sender
purpose: "Encrypts sensitive data with TDF and shares via Iroh P2P data plane"
model: ministral-3b

kas:
  enabled: true
  key_id: "sender-key-1"
  algorithm: "ec:secp256r1"

a2a:
  enabled: true
  discovery:
    mdns: true
---

# Data Sender

Encrypts files using TDF (AES-256-GCM) with attribute-based access control
and stages them to the Iroh P2P network for secure transfer to authorized agents.

## Capabilities

- **tdf_encrypt** - Encrypt data with policy-based access control
- **tdf_share** - Encrypt + stage to Iroh + return ticket for sharing
- **kas.publicKey** - Provide public key for peer encryption

## Data Flow

1. Read sensitive input (code, findings, credentials)
2. Fetch receiver's KAS public key via `kas.publicKey` RPC
3. Encrypt with TDF using ABAC policy attributes
4. Stage encrypted blob to local Iroh node
5. Send ticket to receiver via `tdf.share` RPC
