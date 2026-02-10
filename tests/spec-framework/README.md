# Spec-Driven Test Framework

This directory previously contained shell scripts for spec testing. The framework has been **migrated to xtask**.

## Location

```bash
.cargo/xtask/src/
├── spec_test.rs          # Core library (parsing, coverage, generation)
└── spec_test_cmds.rs     # CLI subcommands
```

## Usage

```bash
# Coverage report
cargo xtask spec-test coverage

# List scenarios
cargo xtask spec-test list

# Find uncovered scenarios
cargo xtask spec-test uncovered

# Generate test stubs
cargo xtask spec-test generate --uncovered-only
```

## Migration Guide

| Old Command | New Command |
|-------------|-------------|
| `./spec-coverage.sh` | `cargo xtask spec-test coverage` |
| `./spec-coverage.sh gossip` | `cargo xtask spec-test coverage --spec gossip` |
| `./spec-test-gen.sh hrm` | `cargo xtask spec-test generate hrm` |
| `./spec-test-gen.sh --missing` | `cargo xtask spec-test uncovered --generate` |
| `./spec-runner.sh --scenario HRM-003` | `cargo test HRM-003` |

## Documentation

See [docs/spec-driven-testing.md](../../docs/spec-driven-testing.md) for full documentation.
