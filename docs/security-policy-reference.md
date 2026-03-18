# Arkavo Edge Security Policy Reference Guide

## Overview

This document maps the Arkavo Edge security architecture to the OWASP Agentic Security Initiative (ASI) 2026 framework and provides implementation details for each security control.

## OWASP ASI 2026 Mapping

### ASI01: Agent Goal Hijacking → Goal & Intent Governance

**Threat**: Attackers manipulate prompts to make agents pursue malicious objectives.

**Arkavo Mitigations**:

| Control | Implementation | Location |
|---------|---------------|----------|
| Constitutional Principles | Immutable core instructions in `goal_governance.constitutional_principles` | Policy YAML |
| Goal Hierarchy | Layered instruction priority system | `goal_governance.goal_hierarchy` |
| Objective Drift Detection | Semantic similarity monitoring | `goal_governance.objective_drift_detection` |
| HITL Triggers | Cryptographic approval for high-risk ops | `goal_governance.hitl_triggers` |

**Code Implementation**:
- `crates/arkavo-orchestrator/src/task_policy_manager.rs` - Policy decision point
- `crates/arkavo-router/src/preflight/moderator.rs` - Pre-flight content moderation

---

### ASI02: Tool Misuse & Exploitation → Execution & Tooling Sandboxing

**Threat**: Agents execute dangerous tools or use tools beyond their intended scope.

**Arkavo Mitigations**:

| Control | Implementation | Location |
|---------|---------------|----------|
| Tool Allowlisting | Signature-verified tool registry | `tool_execution.allowlist` |
| Risk Classification | Low/Medium/High/Critical tool categories | `tool_execution.risk_classification` |
| Sandbox Isolation | Docker/firejail/sandbox-exec | `tool_execution.sandbox` |
| Least Privilege | ABAC tool-to-identity bindings | `tool_execution.least_privilege` |

**Code Implementation**:
- `crates/arkavo-mcp-tools/src/sandbox.rs` - MCP tool sandboxing
- `crates/arkavo-dataflow/src/engine/sandbox.rs` - Transform execution sandbox
- `crates/arkavo-orchestrator/src/task_policy_manager.rs` - Tool risk classification

**Sandbox Backends**:
- **Linux**: Firejail with seccomp, capabilities dropping
- **macOS**: sandbox-exec with custom profiles
- **Docker**: Container isolation with resource limits

---

### ASI04: Agentic Supply Chain Vulnerabilities → Tool AIBOM

**Threat**: Compromised tools or models in the supply chain.

**Arkavo Mitigations**:

| Control | Implementation | Location |
|---------|---------------|----------|
| Digital Signatures | Ed25519 verification for all tools | `tool_execution.allowlist.signature_verification` |
| AIBOM Validation | Model provenance verification | `tool_execution.aibom` |
| Trusted Registries | Allowlist of model sources | `tool_execution.aibom.trusted_sources` |

**Code Implementation**:
- `crates/arkavo-mcp/src/integrity.rs` - MCP tool integrity verification
- `crates/arkavo-dataflow/src/nodes/model_registry.rs` - Model registry with provenance

---

### ASI06: Memory & Context Poisoning → Data Sovereignty Protection

**Threat**: RAG context injection or data leakage between sessions.

**Arkavo Mitigations**:

| Control | Implementation | Location |
|---------|---------------|----------|
| ABAC Encryption | OpenTDF attribute-based encryption | `data_protection.abac` |
| Memory Isolation | Task compartments, user context isolation | `data_protection.memory_protection.isolation` |
| Poisoning Detection | Semantic drift and injection detection | `data_protection.memory_protection.poisoning_detection` |
| DLP | Automatic PII/credential detection | `data_protection.dlp` |

**Code Implementation**:
- `crates/arkavo-tdf/src/abac.rs` - ABAC policy evaluator
- `crates/arkavo-tdf/src/policy.rs` - TDF policy builder
- `crates/arkavo-security/src/data_classification.rs` - Data classification
- `crates/arkavo-validation/src/sanitize.rs` - Log sanitization
- `crates/arkavo-router/src/preflight/moderator.rs` - PII detection

**OpenTDF Attributes**:
```rust
// Standard attribute namespaces
https://arkavo.net/attr/role      // Role-based access
https://arkavo.net/attr/clearance // Security clearance
https://arkavo.net/attr/mcp-tool  // Tool access
https://arkavo.net/attr/agent-id  // Agent identity
https://arkavo.net/attr/organization
```

---

### ASI07: Insecure Inter-Agent Communication → Mesh Security

**Threat**: Unauthorized agents issuing commands or message tampering.

**Arkavo Mitigations**:

| Control | Implementation | Location |
|---------|---------------|----------|
| Cryptographic Attestation | TPM/Secure Enclave verification | `inter_agent_security.attestation` |
| A2A Policy | Per-agent-pair communication rules | `inter_agent_security.a2a_policy` |
| Payload Validation | Schema enforcement, injection detection | `inter_agent_security.payload_validation` |
| Iroh Ticket Validation | P2P blob access control | `inter_agent_security.iroh_mesh` |

**Code Implementation**:
- `crates/arkavo-attestation/src/lib.rs` - Platform attestation
- `crates/arkavo-protocol/src/a2a_policy.rs` - A2A communication policy
- `crates/arkavo-gossip/src/verification.rs` - Gossip message verification

---

### ASI08: Cascading Failures → Blast Radius Control

**Threat**: One agent failure compromises the entire mesh.

**Arkavo Mitigations**:

| Control | Implementation | Location |
|---------|---------------|----------|
| Resource Limits | Token, budget, TTL constraints | `blast_radius_control.resource_limits` |
| Global Kill Switch | Instant capability revocation | `blast_radius_control.kill_switch` |
| Circuit Breakers | Failure isolation | `blast_radius_control.circuit_breakers` |
| Graceful Degradation | Partial functionality maintenance | `blast_radius_control.graceful_degradation` |

**Code Implementation**:
- `crates/arkavo-budget/src/policy.rs` - Budget policy and model selection
- `crates/arkavo-protocol/src/rate_limit.rs` - Rate limiting
- `crates/arkavo-session/src/revocation.rs` - Session revocation

---

### ASI09: Human-Agent Trust Exploitation → Identity & Attestation

**Threat**: Agents impersonating humans or bypassing human oversight.

**Arkavo Mitigations**:

| Control | Implementation | Location |
|---------|---------------|----------|
| HITL Cryptographic Approval | Signature-based authorization | `goal_governance.hitl_triggers` |
| Biometric Authentication | Face ID/Touch ID for critical ops | `goal_governance.hitl_triggers.conditions[].require_biometric` |
| Agent Identity Verification | DID:key with device binding | `inter_agent_security.attestation` |

**Code Implementation**:
- `crates/arkavo-device-identity/src/keypair.rs` - Device identity
- `crates/arkavo-mcp-macos/src/mcp/face_id_control.rs` - Biometric auth

---

### ASI10: Rogue Agents → Constitutional AI & Governance

**Threat**: Agents acting outside their defined scope or objectives.

**Arkavo Mitigations**:

| Control | Implementation | Location |
|---------|---------------|----------|
| Constitutional Principles | Immutable behavioral boundaries | `goal_governance.constitutional_principles` |
| Objective Drift Detection | Continuous goal alignment monitoring | `goal_governance.objective_drift_detection` |
| Orchestrator Exemption Control | Central coordinator oversight | `inter_agent_security.a2a_policy.orchestrator_id` |

---

## Additional Security Controls

### Network Security (NET-*)

| Spec ID | Control | Implementation |
|---------|---------|---------------|
| NET-004 | No Localhost Trust | All requests require auth, no exemptions |
| NET-006 | Host Validation | DNS rebinding protection via `HostValidator` |
| NET-007 | Egress Filtering | SSRF prevention, private IP blocking |
| NET-008 | Ephemeral Tokens | Setup token with automatic revocation |
| NET-010 | Rate Limiting | Per-IP and global rate limiting |

**Code Implementation**:
- `crates/arkavo-protocol/src/security_fixes.rs` - Security vulnerability fixes
- `crates/arkavo-validation/src/url.rs` - URL/egress validation
- `crates/arkavo-protocol/src/rate_limit.rs` - Rate limiting

### Session Security (SESS-*)

| Spec ID | Control | Implementation |
|---------|---------|---------------|
| SESS-007 | Admin Revocation | Immediate session termination |
| SESS-008 | User Logout | Self-initiated session end |
| SESS-009 | Bulk Revocation | Criteria-based session cleanup |

**Code Implementation**:
- `crates/arkavo-session/src/revocation.rs` - Session revocation
- `crates/arkavo-session/src/log_sanitizer.rs` - Log sanitization
- `crates/arkavo-session/src/error_sanitizer.rs` - Error sanitization

### TDF Audit (TDFS-*)

| Spec ID | Control | Implementation |
|---------|---------|---------------|
| TDFS-001 | Cloud Prompt Encryption | AES-256-GCM encrypted audit trail |
| TDFS-002 | Config Encryption | KAS-encrypted configuration bundles |

**Code Implementation**:
- `crates/arkavo-router/src/tdf_audit.rs` - Message encryption audit
- `crates/arkavo-tdf/src/lib.rs` - OpenTDF implementation
- `crates/arkavo-config-encryption/src/kas.rs` - KAS encryption

---

## Security Test Suite

### Running Security Tests

```bash
# Security vulnerability tests
cargo test -p arkavo-protocol --test security_vulnerabilities

# Mock provider security tests
cargo test -p arkavo-cli mock_provider

# E2E security tests
./tests/e2e_security_test.sh
./tests/security_cli_test.sh
./tests/dlp_pii_security_test.sh
```

### Security Test Coverage

| Test File | Coverage |
|-----------|----------|
| `security_vulnerabilities.rs` | CRI-001 through HIGH-005 |
| `arkavo/tests/http_security_integration.rs` | HTTP security headers, TLS |
| `arkavo/tests/security_integration_tests.rs` | End-to-end security flows |

---

## Policy Deployment

### Hot Reload

```bash
# Apply policy changes without restart
arkavo policy apply --file security-policy.yaml --hot-reload
```

### Policy Validation

```bash
# Validate policy before deployment
arkavo policy validate --file security-policy.yaml
```

### Policy Versioning

Policies follow semantic versioning:
- **Major**: Breaking security control changes
- **Minor**: New security features
- **Patch**: Bug fixes, configuration updates

---

## Security Monitoring

### Metrics Export

```yaml
# Prometheus-compatible metrics
arkavo_security_authentication_attempts_total
arkavo_security_authorization_denials_total
arkavo_security_tool_execution_risk_total{risk="critical"}
arkavo_security_dlp_triggers_total{type="pii"}
arkavo_security_drift_detection_events_total
arkavo_security_kill_switch_activations_total
```

### Alerting Rules

```yaml
# Example alert for kill switch activation
- alert: KillSwitchActivated
  expr: arkavo_security_kill_switch_activations_total > 0
  severity: critical
  
- alert: HighPolicyViolationRate
  expr: rate(arkavo_security_policy_violations_total[5m]) > 0.1
  severity: warning
```

---

## Compliance Mapping

| Framework | Arkavo Controls |
|-----------|-----------------|
| **SOC 2** | Audit logging, access controls, monitoring |
| **GDPR** | Data classification, encryption, retention |
| **NIST AI RMF** | Risk governance, measurement, evaluation |
| **ISO 27001** | Security policy, asset management, crypto |

---

## References

- [OWASP ASI 2026](https://owasp.org/www-project-agentic-security/)
- [OpenTDF Specification](https://github.com/opentdf/spec)
- [Arkavo Architecture](AGENTS.md)
- [Security Specifications](../specs/arkavo-edge/)
