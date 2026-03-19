# Architecture Decision Records (ADR) 2026
## Security & Identity Architecture

---

## ADR-001: Adopt ADL as Primary Policy Format

**Status:** Proposed  
**Date:** March 2026  
**Deciders:** Security Architecture Team

### Context
Enterprise security teams require policy formats that integrate with existing GRC tools. Proprietary formats create adoption friction.

### Decision
Adopt the **Agent Definition Language (ADL)** from DIF TAAWG as the primary policy format, with Arkavo-specific extensions.

### Consequences
- **Positive:** Standards compliance, easier enterprise adoption
- **Negative:** Dependency on evolving standard, need ADL parser

### Implementation
See `docs/strategy-pillar-1-adl-cosai-policy.md`

---

## ADR-002: Implement DIF TAAWG DID Methods

**Status:** Proposed  
**Date:** March 2026  
**Deciders:** Identity Team

### Context
Agent identity must be cryptographically verifiable, non-transferable, and work offline.

### Decision
Implement `did:arkavo` method with device binding (TPM/Secure Enclave), plus support for `did:web` (enterprise) and `did:key` (ephemeral).

### Consequences
- **Positive:** Decentralized, no single point of failure
- **Negative:** Complexity of key management, recovery procedures

### Implementation
See `docs/strategy-pillar-2-kya-identity.md`

---

## ADR-003: Bind OpenTDF to W3C Verifiable Credentials

**Status:** Proposed  
**Date:** March 2026  
**Deciders:** Crypto Team

### Context
Zero-trust data access requires strong identity verification. Traditional IAM doesn't provide cryptographic guarantees.

### Decision
Modify OpenTDF KAS to only issue decryption keys when agent presents valid W3C VC matching the data's ABAC policy.

### Consequences
- **Positive:** Cryptographic access control, audit trail
- **Negative:** VC issuance infrastructure needed

### Implementation
See `docs/strategy-pillar-2-kya-identity.md` section "OpenTDF-to-VC Binding"

---

## ADR-004: Tunnel A2A over Iroh P2P

**Status:** Proposed  
**Date:** March 2026  
**Deciders:** Networking Team

### Context
HTTP-based A2A requires cloud connectivity and central brokers. Edge deployments need offline capability.

### Decision
Implement A2A protocol exclusively over Iroh P2P mesh (QUIC-based), with OpenTDF encryption.

### Consequences
- **Positive:** Works offline, NAT traversal, no cloud dependency
- **Negative:** New transport stack, debugging complexity

### Implementation
See `docs/strategy-pillar-3-a2a-iroh-mesh.md`

---

## ADR-005: Replace Docker with eBPF/Wasm Sandboxing

**Status:** Proposed  
**Date:** March 2026  
**Deciders:** Platform Team

### Context
Docker sandboxing is slow (500ms+ startup) and vulnerable to container escape.

### Decision
Use **eBPF** for kernel-level enforcement (network, filesystem) and **WebAssembly (Wasmtime)** for lightweight tool isolation.

### Consequences
- **Positive:** < 5ms startup, formally verified isolation, kernel-level enforcement
- **Negative:** eBPF complexity, limited language support for Wasm

### Implementation
See `docs/strategy-pillar-4-ebpf-wasm-sandbox.md`

---

## ADR-006: Implement Cryptographic HITL

**Status:** Proposed  
**Date:** March 2026  
**Deciders:** Security Architecture Team

### Context
Traditional HITL (web UI clicks) lacks non-repudiation and is vulnerable to session hijacking.

### Decision
Require cryptographic signatures (DID-based) for high-risk action approval, with enterprise wallet integration.

### Consequences
- **Positive:** Non-repudiable, works offline, enterprise PKI compatible
- **Negative:** User friction, wallet dependency

### Implementation
See `docs/strategy-pillar-2-kya-identity.md` section "CryptoHITL"

---

## ADR-007: Adopt CoSAI Controls for Audit Logging

**Status:** Proposed  
**Date:** March 2026  
**Deciders:** Compliance Team

### Context
Enterprise SIEM tools require standardized control mappings for compliance reporting.

### Decision
Map all security events to CoSAI Workstream 4 controls and OWASP ASI 2026 mitigations in audit logs.

### Consequences
- **Positive:** Compliance automation, industry standard alignment
- **Negative:** Maintenance as standards evolve

### Implementation
See `docs/strategy-pillar-1-adl-cosai-policy.md` section "CoSAI Integration"

---

## ADR-008: Use OPA-Wasm for Policy Evaluation

**Status:** Proposed  
**Date:** March 2026  
**Deciders:** Performance Team

### Context
Policy evaluation must be fast (< 1ms) to avoid latency in agent workflows.

### Decision
Use Open Policy Agent (OPA) compiled to WebAssembly, with custom Rust fast-path for hot policies.

### Consequences
- **Positive:** Rego ecosystem, Wasm sandboxed, fast
- **Negative:** OPA learning curve, Wasm overhead

### Implementation
See `docs/strategy-pillar-1-adl-cosai-policy.md` section "OPA-Wasm Policy Engine"

---

## Summary Decision Matrix

| ADR | Decision | Status | Risk | Owner |
|-----|----------|--------|------|-------|
| ADR-001 | Adopt ADL | Proposed | Medium | Security |
| ADR-002 | Implement DIDs | Proposed | Medium | Identity |
| ADR-003 | OpenTDF-VC Binding | Proposed | High | Crypto |
| ADR-004 | A2A over Iroh | Proposed | Medium | Networking |
| ADR-005 | eBPF/Wasm Sandbox | Proposed | High | Platform |
| ADR-006 | CryptoHITL | Proposed | Low | Security |
| ADR-007 | CoSAI Audit Tags | Proposed | Low | Compliance |
| ADR-008 | OPA-Wasm Policies | Proposed | Medium | Performance |

---

*ADRs are living documents. Update as decisions are implemented or changed.*
