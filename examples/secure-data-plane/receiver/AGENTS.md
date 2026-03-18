---
name: data-receiver
purpose: "Receives TDF-encrypted data via Iroh P2P and decrypts using embedded KAS"
model: ministral-3b

kas:
  enabled: true
  key_id: "receiver-key-1"
  algorithm: "ec:secp256r1"
  trusted_roots:
    - did: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
      name: "Demo Root Authority"

a2a:
  enabled: true
  discovery:
    mdns: true
---

# Data Receiver

Accepts TDF-encrypted data shares from other agents via the Iroh P2P data plane,
verifies access through embedded KAS with ABAC policy evaluation, and decrypts.

## Capabilities

- **tdf.share** - Accept encrypted data offers from peers
- **tdf.offers** - List pending encrypted data offers
- **tdf_receive** - Fetch encrypted data from Iroh and save manifest
- **kas.rewrap** - Rewrap TDF keys with delegation verification
- **kas.publicKey** - Provide public key for peer encryption

## Data Flow

1. Receive `tdf.share` RPC with Iroh ticket + metadata
2. List offers via `tdf.offers`
3. Fetch encrypted blob from Iroh using ticket
4. Verify delegation token and ABAC policy via embedded KAS
5. Rewrap encryption key and decrypt payload
