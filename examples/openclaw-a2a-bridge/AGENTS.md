---
name: arkavo-bridge-agent
purpose: "A2A protocol bridge demonstrating TDF encryption, budget enforcement, and preflight policies"
model: ministral-3b

kas:
  enabled: true
  key_id: "bridge-demo-key-1"
  algorithm: "ec:secp256r1"
  trusted_roots:
    - did: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
      name: "Demo Root Authority"

preflight:
  policies:
    - id: block_pii
      features: [InputContainsPII]
      action: block
      description: "Blocks SSN, credit card numbers, and email addresses"
      enabled: true
    - id: block_shell_commands
      features: [InputContainsShellCommands]
      action: block
      description: "Blocks shell commands like sudo, rm, chmod"
      enabled: true

a2a:
  enabled: true
  discovery:
    mdns: true
---

# Arkavo Bridge Agent

This agent serves as Arkavo's side of the A2A protocol bridge to OpenClaw. It
combines KAS encryption, preflight policy enforcement, and budget tracking in a
single agent configuration.

## Capabilities

- **TDF Encryption** via KAS (key: `bridge-demo-key-1`, algorithm: `ec:secp256r1`)
- **Preflight Policies** block PII and shell commands before they reach the LLM
- **Budget Enforcement** caps session spending at $1.00
- **Local Inference** using `ministral-3b` at zero cost

## Why This Matters

When OpenClaw sends a coding task through the bridge, Arkavo processes it with:

1. Preflight scan (PII, shell injection) before any inference
2. TDF-encrypted context that never leaves the machine in plaintext
3. Budget tracking that prevents runaway costs
4. Local model inference with no cloud dependency

OpenClaw's default path sends the same task to a cloud API in plaintext with no
budget cap, no PII detection, and no offline fallback.

## A2A Methods

- `kas.publicKey` - Retrieve the KAS public key for TDF encryption
- `kas.rewrap` - Rewrap TDF encryption keys with ABAC policy enforcement
- `message/send` - Submit a coding task for processing
