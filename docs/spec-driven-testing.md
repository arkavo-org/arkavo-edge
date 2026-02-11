# Spec-Driven Testing System

A Rust-based framework that connects BDD specifications to test implementations.

## Overview

The spec-driven testing system bridges the gap between **BDD specifications** (`specs/arkavo-edge/*.spec.yaml`) and **Rust test implementations**.

```
┌─────────────────────────────────────────────────────────────────┐
│                     SPEC TEST SYSTEM                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  specs/arkavo-edge/                                            │
│  ├── gossip-protocol.spec.yaml  ────┐                           │
│  ├── hrm.spec.yaml                │                           │
│  └── ...                          │                           │
│                                   ▼                           │
│  ┌─────────────────────────────────────────┐                  │
│  │     xtask spec-test subcommand         │                  │
│  │  ┌─────────────┐ ┌─────────────────┐   │                  │
│  │  │ SpecParser  │ │ TestDiscovery   │   │                  │
│  │  └─────────────┘ └─────────────────┘   │                  │
│  │  ┌─────────────┐ ┌─────────────────┐   │                  │
│  │  │ CoverageAnalyzer│ │ TestGenerator│   │                  │
│  │  └─────────────┘ └─────────────────┘   │                  │
│  └─────────────────────────────────────────┘                  │
│                     │                                          │
│         ┌───────────┴───────────┐                             │
│         ▼                       ▼                             │
│  Coverage Reports         Test Stubs                          │
│  ─────────────────        ──────────                          │
│  • Pretty tables          • Auto-generated                    │
│  • JSON/Markdown          • Spec-linked                       │
│  • CI integration         • Ready to implement                │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Quick Start

```bash
# Show coverage summary
cargo xtask spec-test coverage

# List all scenarios with test counts
cargo xtask spec-test list --with-tests

# Find uncovered scenarios
cargo xtask spec-test uncovered

# Generate test stubs for uncovered scenarios
cargo xtask spec-test uncovered --generate
```

## Commands

### `coverage` - Coverage Report

Shows how many scenarios have test implementations:

```bash
# Basic coverage
cargo xtask spec-test coverage

# Detailed per-scenario view
cargo xtask spec-test coverage --detailed

# Filter by spec
cargo xtask spec-test coverage --spec gossip-protocol
```

**Output:**
```
Spec Coverage Report
====================

Spec                      Total  Covered       Status
------------------------------------------------------------
hrm                           6       3       🟡 Partial
gossip-protocol               8       8       🟢 Full
orchestrator                 11       0       🔴 Missing
------------------------------------------------------------
TOTAL                        25      11

Coverage: 44.0%
```

### `list` - List Scenarios

Shows all BDD scenarios from specs:

```bash
# List all
cargo xtask spec-test list

# With test counts
cargo xtask spec-test list --with-tests

# Filter by spec
cargo xtask spec-test list --spec hrm
```

**Output:**
```
▶ hrm.spec - Hierarchical Reasoning Model Orchestration
  • HRM-001 [critical] Create conductor with task store (no tests)
  • HRM-002 [critical] Create task with budget (✓ 1 tests)
  • HRM-003 [critical] Execute burst with bounded contract (✓ 5 tests)
```

### `uncovered` - Find Gaps

Shows scenarios without tests:

```bash
# Show only
cargo xtask spec-test uncovered

# Generate stubs
cargo xtask spec-test uncovered --generate
```

### `generate` - Create Test Stubs

Generates Rust test stubs from specs:

```bash
# All specs
cargo xtask spec-test generate

# Only uncovered
cargo xtask spec-test generate --uncovered-only

# Specific spec
cargo xtask spec-test generate hrm

# Custom output
cargo xtask spec-test generate --output tests/generated/
```

**Generated stub:**
```rust
/// Covers HRM-003: Execute burst with bounded contract
/// Spec: specs/arkavo-edge/hrm.spec.yaml
/// Criticality: critical
#[tokio::test]
async fn test_hrm_003_execute_burst_with_bounded_contract() {
    // TODO: Arrange - Set up preconditions
    // Given: Task active, BurstContract defined
    
    // TODO: Act - Execute the action  
    // When: burst_execute called
    
    // TODO: Assert - Verify expected outcomes
    // Then: Budget enforced, Execution bounded, Result or partial result returned
    
    unimplemented!("Test stub for HRM-003 - implement based on spec");
}
```

## Linking Specs to Tests

Tests are automatically linked to specs by referencing scenario IDs in doc comments:

```rust
/// Covers HRM-003: Budget enforcement with cost tracking
#[tokio::test]
async fn test_budget_enforcement_cost() {
    // Test implementation
}

/// Covers GOSSIP-001, GOSSIP-002: Epidemic broadcast and verification
#[tokio::test]
async fn test_gossip_basic_flow() {
    // Test implementation
}
```

The `TestDiscovery` module scans for patterns like:
- `Covers HRM-003:` - Single scenario
- `Covers GOSSIP-001, GOSSIP-002:` - Multiple scenarios
- `scenario HRM-003` - Alternative format

## Architecture

### Core Components

```rust
// Parse YAML specs
let specs = SpecParser::parse_all_specs(Path::new("specs/arkavo-edge"))?;

// Discover Rust tests
let tests = TestDiscovery::new()?.discover_tests(Path::new("crates"))?;

// Analyze coverage
let report = CoverageAnalyzer::analyze(specs, tests);

// Generate stubs
let stub = TestGenerator::generate_stub(&spec, &scenario);
```

### Data Types

| Type | Description |
|------|-------------|
| `Spec` | A parsed specification file |
| `Scenario` | BDD scenario with id, name, given/when/then |
| `Test` | Discovered Rust test function |
| `CoverageReport` | Aggregated coverage statistics |
| `CoverageStatus` | Covered/Partial/Missing |

## CI Integration

```yaml
# .github/workflows/spec-coverage.yml
name: Spec Coverage

on: [push, pull_request]

jobs:
  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Check Spec Coverage
        run: cargo xtask spec-test coverage
        
      - name: Generate Report
        run: |
          cargo xtask spec-test coverage --detailed > coverage.txt
          
      - name: Check Threshold
        run: |
          # Fail if critical scenarios are uncovered
          UNCOVERED_CRITICAL=$(cargo xtask spec-test uncovered | grep -c "\[CRITICAL\]" || true)
          if [ "$UNCOVERED_CRITICAL" -gt 0 ]; then
            echo "Error: $UNCOVERED_CRITICAL critical scenarios lack tests"
            exit 1
          fi
```

## Workflow: Adding a New Spec Scenario

1. **Add scenario to spec:**
   ```yaml
   # specs/arkavo-edge/my-component.spec.yaml
   scenarios:
     - id: MYCOMP-007
       name: Handle edge case X
       criticality: high
       given: [System in state Y]
       when: Event Z occurs
       then: [System transitions to state W]
   ```

2. **Generate test stub:**
   ```bash
   cargo xtask spec-test generate my-component
   ```

3. **Implement the test:**
   ```rust
   // In crates/arkavo-my-component/tests/
   /// Covers MYCOMP-007: Handle edge case X
   #[tokio::test]
   async fn test_mycomp_007_handle_edge_case_x() {
       // Arrange
       let system = System::in_state_y().await;
       
       // Act
       let result = system.handle(event_z).await;
       
       // Assert
       assert!(result.is_in_state_w());
   }
   ```

4. **Verify coverage:**
   ```bash
   cargo xtask spec-test coverage --spec my-component
   ```

## Benefits

| Before | After |
|--------|-------|
| Manual spec tracking | Automated coverage analysis |
| Specs drift from tests | Bidirectional linking |
| No visibility into gaps | Gap reports with stub generation |
| Ad-hoc test naming | Spec-driven test naming |
| Documentation in two places | Single source of truth |

## Future Enhancements

- [ ] Watch mode for continuous coverage monitoring
- [ ] HTML report generation
- [ ] Integration with `cargo test` for spec filtering
- [ ] Custom derive macros: `#[spec_test(HRM-003)]`
- [ ] Coverage trends over time
- [ ] PR comments with coverage diff
