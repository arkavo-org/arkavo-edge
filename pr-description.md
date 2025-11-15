## Summary

Implements a secure configuration distribution system where a central orchestrator packages and distributes encrypted configuration bundles to multiple agents using OpenTDF (Open Trusted Data Format) for encryption, policy enforcement, and access control.

Closes #311

## Architecture

- **Configuration Bundle Packager**: Orchestrator-side bundle creation with settings, roles, entitlements, and secrets
- **OpenTDF Encryption**: End-to-end encryption with attribute-based access control policies using opentdf-rs
- **Public/Private Key Authentication**: Agent identity verification using ECDSA P-256 asymmetric cryptography
- **Centralized KAS**: Key Access Service at https://100.arkavo.net/kas/v2/rewrap for policy enforcement and key unwrapping
- **Secure Distribution**: Signed requests with public/private key pairs + JWT authentication

## Implementation Plan

### Phase 1: Core Infrastructure (Week 1-2) - 40 hours
**New Crates:**
- ✅ `arkavo-config-bundle` - Configuration bundle data structures (< 400 LOC)
- ✅ `arkavo-config-encryption` - OpenTDF integration with opentdf-rs (< 400 LOC)

**Deliverables:**
- ✅ Configuration bundle structure with settings, roles, entitlements, secrets
- ✅ OpenTDF encryption using opentdf-rs PolicyBuilder and Tdf::encrypt
- ✅ Policy management and attribute definitions
- ✅ Unit tests with >85% coverage
- ✅ KAS configuration module with production URL (https://100.arkavo.net/kas/v2/rewrap)
- ✅ decrypt_bundle_async() method ready for KAS client integration
- ✅ Complete example demonstrating encryption workflow
- ✅ Comprehensive README with architecture diagrams and usage
- ⚠️ Full decrypt requires KAS OAuth token (Phase 3)

### Phase 2: Distribution System (Week 3-4) - 40 hours
**New Crates:**
- ✅ `arkavo-config-distribution` - Orchestrator-side distribution logic (< 400 LOC)

**Deliverables:**
- Bundle management API (create, update, delete, distribute)
- Public/private key authentication for agents
- Agent registration with public key storage
- Request signature verification
- Integration with existing Agent Registry

### Phase 3: Agent Integration (Week 5) - 20 hours
**New Crates:**
- ✅ `arkavo-config-client` - Agent-side configuration client (< 400 LOC)

**Deliverables:**
- Configuration fetching with signed requests
- OpenTDF decryption via KAS
- Configuration application (settings, secrets, entitlements)
- Automatic configuration on agent startup
- End-to-end integration tests

### Phase 4: Security & Audit (Week 6) - 30 hours
**Deliverables:**
- Comprehensive audit logging (tamper-evident)
- Access control policy enforcement
- Secrets rotation with grace periods
- Security testing and penetration testing
- Policy decision logging

### Phase 5: CLI & UI (Week 7) - 25 hours
**Deliverables:**
- CLI commands for bundle management
- Web UI for configuration management
- Audit log viewer
- User documentation and guides
- Tutorial examples

### Phase 6: Testing & Documentation (Week 8) - 20 hours
**Deliverables:**
- Performance testing (1000+ concurrent agents)
- Load testing and optimization
- Complete technical documentation
- Security best practices guide
- Troubleshooting guide

**Total Estimated Effort:** 175 hours (8 weeks)

## Key Features

### Security
- **End-to-End Encryption**: AES-256-GCM via OpenTDF
- **Attribute-Based Access Control (ABAC)**: Fine-grained policies based on agent attributes
- **Public/Private Key Authentication**: ECDSA P-256 for agent identity verification and request signing
- **Centralized Key Management**: arkavo-rs KAS with automated rotation
- **JWT Authentication**: RS256 signed tokens with agent attributes
- **Comprehensive Audit Trail**: Tamper-evident logs for compliance (SOC 2, ISO 27001, GDPR)
- **Secrets Rotation**: Automated rotation with configurable grace periods
- **Dynamic Policy Enforcement**: Real-time access control with revocation capabilities

### Configuration Bundle Structure
Configuration bundles contain settings, roles, entitlements, and encrypted secrets with OpenTDF policies.

### Public/Private Key Authentication Flow
1. **Agent Registration**: Agent generates key pair, registers public key with orchestrator
2. **Request Signing**: Agent signs all requests with private key
3. **Signature Verification**: Orchestrator verifies signature using registered public key
4. **JWT Token**: Additional JWT token with agent attributes for KAS authorization
5. **KAS Decryption**: Agent authenticates to KAS with JWT, receives decryption key if authorized

## User Workflows

### Administrator: Create Configuration
```bash
arkavo config create \
  --target role:search-agent \
  --secret GOOGLE_API_KEY=sk-xxx \
  --entitlement mcp_tool:web-search:execute \
  --attribute agent.role=search-agent
```

### Agent: Fetch Configuration (Automatic)
```bash
arkavo agent run --id search-agent-001
# Automatically:
# 1. Signs request with private key
# 2. Requests configuration from orchestrator
# 3. Decrypts bundle using OpenTDF + KAS
# 4. Applies configuration (settings, secrets, entitlements)
```

### Auditor: Review Access
```bash
arkavo audit config-access --bundle bundle-id --since 2025-01-01
# Returns detailed logs with policy decisions
```

### DevOps: Rotate Secrets
```bash
arkavo config rotate-secret \
  --bundle bundle-id \
  --secret GOOGLE_API_KEY \
  --new-value sk-new-xxx \
  --grace-period 1h
```

## Integration Points

- **Agent Registry** (`arkavo-protocol/agent_registry.rs`): Extended with agent attributes and public key storage
- **Task Planner** (`arkavo-protocol/task_planner.rs`): Uses configuration bundles for capability validation
- **Authorization Module** (`arkavo-authorization`): Integrates OpenTDF authorization checks
- **Protocol Server** (`arkavo-protocol/server.rs`): Adds configuration distribution endpoints

## Dependencies

- **opentdf-rs**: https://github.com/arkavo-org/opentdf-rs (OpenTDF Rust implementation)
- **arkavo-rs**: https://github.com/arkavo-org/arkavo-rs (KAS implementation)
- **ring**: Cryptographic operations (ECDSA signing/verification)
- **jsonwebtoken**: JWT token generation and validation

## Acceptance Criteria

### Core Functionality
- [ ] Bundle creation with encryption
- [ ] OpenTDF integration with KAS
- [ ] Public/private key authentication
- [ ] Signed request verification
- [ ] Secure distribution with signature validation
- [ ] Attribute-based access control
- [ ] Configuration decryption and application
- [ ] Comprehensive audit logging

### Security Requirements
- [ ] Encryption at rest and in transit
- [ ] Zero plaintext secrets in logs
- [ ] Tamper-evident audit trail
- [ ] Dynamic access revocation
- [ ] Policy enforcement at KAS
- [ ] Key rotation support
- [ ] Request signature verification

### Performance Requirements
- [ ] Bundle creation: < 100ms
- [ ] Distribution end-to-end: < 500ms
- [ ] KAS response: < 50ms
- [ ] Agent startup overhead: < 2s
- [ ] Support 1000+ concurrent agents

### Testing Requirements
- [ ] Unit tests >85% coverage
- [ ] Integration tests (end-to-end)
- [ ] Security tests (penetration testing)
- [ ] Performance tests (load testing)
- [ ] Failure handling tests

## Documentation

Complete technical specification available in: `secure-agent-config-proposal.md`

Includes:
- Detailed architecture and system flow
- Complete Rust code examples
- JSON schemas for all data structures
- Security threat model and mitigations
- User stories with acceptance criteria
- API endpoint specifications
- Risk analysis and mitigation strategies

## Success Metrics

### Adoption
- 90% of agents using secure config within 3 months
- 100+ configuration bundles created in first month

### Security
- <1% unauthorized access attempts
- 0 successful policy bypasses
- 100% audit coverage
- <1 hour incident response time

### Performance
- <500ms p95 configuration fetch
- <50ms p95 KAS response
- >99.9% system availability