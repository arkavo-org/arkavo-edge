# AAA Testing Framework Implementation

## Overview

This document describes the completed AAA (Adversarial, Algebraic, Architectural) testing framework for Arkavo Edge, implementing the strategy from the testing quality review.

## Quick Stats

| Category | Test Files | Tests | Coverage Target |
|----------|-----------|-------|-----------------|
| **A**dversarial | 4 | 20+ | 80% mutation score |
| **A**lgebraic | 2 | 14 | 1000+ generated inputs each |
| **A**rchitectural | 3 | 36 | All protocol contracts |
| **Total** | 9 | 70+ | Behavioral confidence |

---

## 1. Adversarial Testing (A)

### Mutation Testing

**Configuration**: `.mutants.toml`

Targets critical security crates:
- `arkavo-crypto`: Signature verification, key derivation
- `arkavo-authorization`: Policy evaluation
- `arkavo-tdf`: Encryption/decryption
- `arkavo-protocol`: Protocol parsing
- `arkavo-gossip`: Distributed consensus

**Running**:
```bash
# Test all critical crates
cargo mutants --config-file .mutants.toml -p arkavo-crypto -p arkavo-tdf

# Target specific file
cargo mutants -p arkavo-crypto --file src/lib.rs
```

**Thresholds**:
- Critical crates: 80% mutation score required
- Other crates: 60% mutation score recommended

### Fault Injection

**Golden Dataset** (`tests/golden_dataset/src/fault_injection.rs`):

| Scenario | Spec ID | Description |
|----------|---------|-------------|
| Network Partition | GOSSIP-008 | Tests gossip resilience during partition |
| Clock Skew | REG-005 | Tests token expiration with skewed clocks |
| KAS Unavailability | TDF-002 | Tests TDF operations without KAS |
| Cascading Failure | ADV-CASCADE | Tests circuit breaker activation |
| Byzantine Fault | ADV-BYZANTINE | Tests consensus with malicious agents |

**Network Fault Tests** (`crates/arkavo-gossip/tests/network_faults.rs`):

```rust
test_network_partition_convergence()  // Partitions heal correctly
test_message_deduplication()          // Duplicate packet handling
test_high_packet_loss_recovery()      // Gossip retry behavior
test_sequential_partitions()          // Multiple partition/recovery cycles
test_byzantine_node_isolation()       // Malicious node containment
test_latency_under_delay()            // Delay tolerance
test_total_network_outage_recovery()  // Complete network failure
test_mesh_message_propagation()       // Basic gossip propagation
```

---

## 2. Algebraic Testing (AA)

### Property-Based Tests

**Crypto Properties** (`crates/arkavo-crypto/tests/properties.rs`):

| Invariant | Property | Generators |
|-----------|----------|------------|
| CRYPTO-INV-001 | Signature determinism | 32-byte seeds, variable messages |
| CRYPTO-INV-002 | Key serialization round-trip | Random 32-byte seeds |
| CRYPTO-INV-003 | Public key consistency | Random 32-byte seeds |
| CRYPTO-INV-004 | Signature verification | Random keys and messages |
| CRYPTO-INV-005 | Tampered message detection | Random message pairs |
| CRYPTO-INV-006 | DID:key round-trip | Random keys |
| CRYPTO-INV-007 | Base64 round-trip | Random keys |
| CRYPTO-INV-011 | ECDH symmetry | Random key pairs |
| CRYPTO-INV-012 | KAS key round-trip | Random 32-byte seeds |
| CRYPTO-INV-013 | SEC1 encoding round-trip | Random keys |
| CRYPTO-INV-014 | ECDH uniqueness | Random key triples |
| CRYPTO-EDGE-001 | Empty message signing | Random keys |
| CRYPTO-EDGE-002 | Large message signing | 10KB messages |
| CRYPTO-EDGE-003 | Invalid signature rejection | Bad signature lengths |

**Running**:
```bash
cargo test -p arkavo-crypto --test properties

# Run with more examples (slower, more thorough)
PROPTEST_CASES=10000 cargo test -p arkavo-crypto --test properties
```

**TDD Approach**:
1. **RED**: Write property test that defines expected behavior
2. **GREEN**: Ensure existing implementation satisfies property
3. **REFACTOR**: Add shrinkers and custom strategies for better error messages

---

## 3. Architectural Testing (AAA)

### A2A Protocol Contracts

**Contract Tests** (`crates/arkavo-protocol/tests/a2a_contracts.rs`):

| Contract | Verification |
|----------|--------------|
| TaskRequest schema | Required fields present, correct types |
| TaskResponse schema | task_id and status present |
| TaskStatus round-trip | All variants serialize correctly |
| AgentCard required fields | name, url, version, capabilities |
| AgentCard round-trip | All fields preserved |
| MessagePart type tags | type field on each part |
| AgentBroadcast schema | agent_id and broadcast_type |
| DiscoverFeatures schema | kebab-case feature types |
| Error handling | Invalid JSON doesn't panic |
| Missing fields | Proper deserialization errors |
| Forward compatibility | Unknown fields ignored |
| ChatOpenRequest schema | Optional fields work |
| MetricsAck schema | Nested structures serialize |
| Complex nested round-trip | Deep structures preserved |

### Network Boundary Tests

**Network Fault Injection** (`crates/arkavo-gossip/tests/network_faults.rs`):

Test infrastructure provides:
- `TestMesh`: Simulated gossip network
- `TestNode`: Individual node with message tracking
- Partition simulation
- Packet loss/duplication
- Latency injection
- Byzantine node simulation

### Spec-Linked Testing

All tests linked to spec scenarios:

```rust
/// Verifies: GOSSIP-008
/// Invariants: Partition detection, eventual consistency
#[test]
fn test_network_partition_convergence() { ... }

/// Verifies: CRYPTO-003
/// Invariants: Key serialization round-trip
#[test]
fn prop_key_serialization_roundtrip(...) { ... }
```

---

## Running the Full Suite

### Quick Check (Development)
```bash
cargo test -p arkavo-crypto --test properties -q
cargo test -p arkavo-protocol --test a2a_contracts -q
cargo test -p arkavo-gossip --test network_faults -q
cargo test -p golden-dataset -q
```

### Full AAA Suite (CI)
```bash
# Algebraic (property-based) - 1000+ examples each
cargo test -p arkavo-crypto --test properties

# Architectural (contract + boundary)
cargo test -p arkavo-protocol --test a2a_contracts
cargo test -p arkavo-gossip --test network_faults

# Adversarial (fault injection)
cargo test -p golden-dataset

# Mutation testing (critical crates)
cargo mutants --config-file .mutants.toml \
  -p arkavo-crypto \
  -p arkavo-authorization \
  -p arkavo-tdf
```

---

## Quality Checklist

For every new test, verify:

- [ ] **Mutation**: Would a flipped operator be caught? (Adversarial)
- [ ] **Property**: Does this hold for 1000+ inputs? (Algebraic)
- [ ] **Contract**: Is this testing the boundary/interface? (Architectural)
- [ ] **Spec Link**: Is this linked to a spec scenario?
- [ ] **Realistic**: Would this catch a real failure mode?

---

## TDD Workflow

```
🔴 RED: Write failing test
   - Define expected behavior clearly
   - Use property-based testing for invariants
   - Use example-based for specific edge cases

🟢 GREEN: Make it pass
   - Implement minimal code
   - Don't optimize yet
   - Verify with mutation testing

🔵 REFACTOR: Improve
   - Extract common test patterns
   - Add custom proptest strategies
   - Improve error messages
```

---

## Future Work

1. **Mutation Testing CI Integration**:
   ```yaml
   - name: Mutation Testing
     run: cargo mutants --config-file .mutants.toml
     continue-on-error: false
   ```

2. **Property Test Coverage**:
   - Add proptest for TDF policy parsing
   - Add state machine tests for chat sessions
   - Add router decision property tests

3. **Fuzzing Integration**:
   - A2A message parsing (completed)
   - TDF policy parsing (completed)
   - JSON-RPC request parsing (existing)

4. **Contract Test Expansion**:
   - Multi-version protocol compatibility
   - Binary protocol contracts (if applicable)
   - Schema evolution tests

---

## Files Added/Modified

### New Files
- `.mutants.toml` - Mutation testing configuration
- `crates/arkavo-crypto/tests/properties.rs` - Property-based tests
- `crates/arkavo-protocol/tests/a2a_contracts.rs` - Protocol contract tests
- `crates/arkavo-gossip/tests/network_faults.rs` - Network fault injection
- `tests/golden_dataset/src/fault_injection.rs` - Fault scenarios

### Modified Files
- `Cargo.toml` - Added proptest to workspace
- `crates/arkavo-crypto/Cargo.toml` - Added proptest dev-dependency
- `crates/arkavo-gossip/Cargo.toml` - Added test dependencies
- `tests/golden_dataset/src/lib.rs` - Added fault_injection module
- `tests/golden_dataset/src/adversarial_runner.rs` - Fixed latency field
- `tests/golden_dataset/src/runner.rs` - Fixed latency field

---

## Success Metrics

| Metric | Before | After | Target |
|--------|--------|-------|--------|
| Property-based tests | 0 | 14 | 20+ |
| Contract tests | 0 | 22 | 30+ |
| Fault injection scenarios | 3 | 8 | 15+ |
| Mutation score | 0% | TBD | 80% |
| Spec-linked tests | 0% | 100% | 100% |

The AAA framework is now ready for CI integration and provides behavioral confidence beyond line coverage.
