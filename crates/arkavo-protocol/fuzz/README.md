# Fuzz Testing for arkavo-protocol

This directory contains fuzz tests for the arkavo-protocol crate.

## Prerequisites

Install cargo-fuzz:
```bash
cargo install cargo-fuzz
```

Note: Fuzzing requires nightly Rust.

## Running Fuzz Tests

### Rate Limiter Fuzzing

Tests the IP rate limiter with random IPs and configurations:

```bash
cargo +nightly fuzz run fuzz_rate_limit
```

This tests:
- Various rate limit configurations
- IPv4 and IPv6 addresses
- LRU eviction under load
- Entry count bounds checking
- TTL-based cleanup

### JSON-RPC Parsing

Tests JSON-RPC request/response parsing:

```bash
cargo +nightly fuzz run fuzz_json_rpc
```

This tests:
- Malformed JSON handling
- Round-trip serialization
- Request/response invariants
- Unknown method handling

### Concurrent Rate Limiting

Tests concurrent access to the rate limiter:

```bash
cargo +nightly fuzz run fuzz_rate_limit_concurrent
```

This tests:
- Multi-threaded access patterns
- Race conditions in eviction
- Concurrent cleanup operations
- Mixed IP access patterns

## Running with Time Limit

For CI or quick testing:

```bash
cargo +nightly fuzz run fuzz_rate_limit -- -max_total_time=600
```

This runs for 10 minutes maximum.

## Reproducing Crashes

If a crash is found, it will be saved in `fuzz/artifacts/`. To reproduce:

```bash
cargo +nightly fuzz run fuzz_rate_limit artifacts/fuzz_rate_limit/crash-<hash>
```

## Coverage

To generate coverage reports:

```bash
cargo +nightly fuzz coverage fuzz_rate_limit
cargo +nightly fuzz coverage fuzz_json_rpc
cargo +nightly fuzz coverage fuzz_rate_limit_concurrent
```

## CI Integration

Fuzz tests run automatically via `.github/workflows/nightly.yaml`:
- Nightly at 2 AM UTC
- On pull requests modifying `arkavo-protocol` or `arkavo-tdf`
- Via manual workflow dispatch

Each target runs in parallel for 10 minutes. Corpus is cached between runs for incremental coverage. Crash artifacts are uploaded on failure with 30-day retention.

A separate job checks the fuzz `Cargo.lock` for outdated dependencies to prevent vulnerability blind spots in Dependabot scanning.