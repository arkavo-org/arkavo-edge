# CI Optimization Guide

This document describes the GitHub Actions CI architecture for arkavo-edge, including design decisions, optimizations, and performance targets.

## Overview

The CI workflow is designed for:
- **Speed**: All test groups complete in <10 minutes
- **Cost efficiency**: Minimize expensive macOS/Windows runner usage
- **Parallelization**: Maximum concurrent execution
- **Early failure detection**: Fast feedback on build issues

## Architecture

### 1. Test & Lint (Parallel, Ubuntu)

Four parallel test groups run simultaneously on cheap Ubuntu runners:

```yaml
test-and-lint:
  strategy:
    fail-fast: false
    matrix:
      group:
        - core        # arkavo-llm, arkavo-memory, arkavo-budget, arkavo-context, arkavo-encryption, arkavo-mcp-core
        - protocol    # arkavo-protocol, arkavo-observability, arkavo-mcp-tools, arkavo-workspace
        - llm-heavy   # arkavo-router (no llama-cpp), arkavo-dataflow, arkavo-terminal
        - ui-heavy    # arkavo-ui-generator, arkavo-agui (no llama-cpp)
```

**Design decisions:**
- **Combined test + clippy**: Run tests first (full codegen), then clippy reuses artifacts (check-mode is nearly free)
- **Feature exclusions**: Exclude llama-cpp from arkavo-router and arkavo-agui to avoid CMake C++ builds (~10-15 min) and HuggingFace model downloads (~170MB)
- **Mold linker**: Fast linking for test binaries
- **sccache + rust-cache**: Shared caches across all groups
- **fail-fast: false**: All groups run even if one fails (better visibility)

**Performance targets:**
- core: ~3-4 minutes
- protocol: ~8 minutes
- llm-heavy: ~9 minutes
- ui-heavy: ~4 minutes

All groups complete in <10 minutes.

### 2. Build Pipeline (Sequential)

```
test-and-lint
    ↓
build-linux (2 parallel: glibc + musl)
    ↓
build-macos & build-windows (parallel, only if Linux succeeds)
```

**Dependency chain rationale:**
- **Linux first**: Cheaper and faster than macOS/Windows
- **Fail fast**: If Linux builds fail, don't waste expensive runner time
- **macOS/Windows parallel**: Both depend on Linux, can run concurrently

### 3. Smoke Tests

Lightweight tests on actual binaries:
- **smoke-test-linux**: Tests musl (Alpine) and glibc (Ubuntu) binaries
- **smoke-test-macos**: Tests macOS ARM64 binary with bundled ONNX Runtime
- **smoke-test-windows**: Tests Windows binary

All smoke tests only download pre-built artifacts (no compilation).

### 4. Validation Jobs (Parallel)

Run independently on Ubuntu:
- **validate-openrpc**: Schema validation
- **validate-doc-snippets**: Documentation code examples
- **validate-xtask-schema**: Schema generation check
- **check-schema-backwards-compatibility**: Breaking change detection
- **zero-config-check**: No hardcoded paths or required env vars
- **no-openssl-check**: Verify rustls usage (musl compatibility)

## Key Optimizations

### 1. Feature Flag Management

**Problem**: arkavo-agui and arkavo-router include llama-cpp in default features, causing:
- CMake C++ compilation (~10-15 min)
- HuggingFace model downloads (~170MB)
- ~20GB disk space consumption

**Solution**: Test/clippy with minimal features:
```bash
# arkavo-router
cargo test -p arkavo-router --no-default-features --features llm-remote,gemini

# arkavo-agui
cargo test -p arkavo-agui --no-default-features --features mdns
```

**Impact**: Saves ~15-20 minutes per CI run, eliminates disk space issues.

### 2. macOS-Specific Tests in Build Job

**Problem**: Separate regression-tests job = extra macOS runner (~10 min)

**Solution**: Merged Issue #114 regression test into build-macos as a pre-build step:
```yaml
build-macos:
  steps:
    - name: Test Issue #114 - No Xcode Prompts  # macOS-specific test
      run: cargo test -p arkavo-mcp-macos --no-default-features --test regression_issue_114

    - name: Build  # Main build
      run: cargo build --release --target aarch64-apple-darwin
```

**Impact**: Eliminates 1 macOS runner, saves ~10 minutes.

### 3. Artifact Reuse (Test → Clippy)

**Design**: Run tests first, then clippy in same job:
- Tests produce full codegen artifacts
- Clippy runs in check-mode and reuses artifacts (~1-2 min)
- Total time < separate jobs by ~5-8 minutes

### 4. Cache Strategy

**rust-cache**: Shared across jobs with common prefix keys:
```yaml
shared-key: "v1-${{ runner.os }}"
prefix-key: "test-lint-${{ matrix.group }}"
```

**sccache**: For C++ compilation (CMake, llama.cpp when needed)

## Cost Optimization

### Runner Usage (per PR)

**Before optimization:**
- Ubuntu: 4 test groups + validation jobs
- macOS: 2 builds + 1 regression + 1 smoke = 4 runners
- Windows: 1 build + 1 smoke = 2 runners

**After optimization:**
- Ubuntu: 4 test groups + validation jobs (unchanged)
- macOS: 1 build (includes regression) + 1 smoke = 2 runners (-50%)
- Windows: 1 build + 1 smoke = 2 runners (unchanged)

**Savings**: 1 macOS runner eliminated (~10 min × $0.08/min = ~$0.80 per PR)

### Time Savings

- Feature exclusions: ~15-20 min
- Regression merge: ~10 min
- **Total**: ~25-30 minutes faster per CI run

## Release Readiness

Final check depends on all critical jobs:
```yaml
release-readiness:
  needs:
    - version-check
    - format
    - test-and-lint
    - smoke-test-linux
    - smoke-test-macos
    - binary-smoke-test
    - performance-check
    - validate-openrpc
    - validate-doc-snippets
    - zero-config-check
    - validate-xtask-schema
    - no-openssl-check
```

All jobs must pass before PR is ready for merge.

## Troubleshooting

### Test Group Timing Out (>10 min)

1. Check if llama-cpp feature leaked in:
   ```bash
   cargo tree -p <crate> --no-default-features --features <features>
   ```

2. Verify no model downloads happening:
   ```bash
   grep -r "hf_hub" crates/<crate>/src/
   ```

3. Check for heavy dependencies:
   ```bash
   cargo tree -p <crate> -e normal --depth 1
   ```

### Disk Space Errors

If disk space errors occur despite optimizations:
1. Check for feature leakage (llama-cpp, candle)
2. Verify GitHub Actions runner has ~14GB free initially
3. Consider adding cleanup step (last resort, costs 5 min)

### macOS Build Failures

1. Check if regression test passed (Issue #114)
2. Verify idb-companion installation succeeded
3. Check ONNX Runtime download/extraction

## Future Optimizations

Potential improvements:
- **Nextest**: Faster test execution (already documented for local dev)
- **Split large tests**: If any group exceeds 10 min consistently
- **Dependency pre-building**: Cache common heavy dependencies
- **Matrix expansion**: Further parallelize large test groups

## Metrics

Target metrics (enforced):
- Test groups: <10 minutes each
- Binary size: <60MB
- Test coverage: ≥85%
- All files: <400 lines

Current performance (as of 2025-01-14):
- core: ~3m21s ✅
- protocol: ~8m31s ✅
- llm-heavy: ~9m49s ✅
- ui-heavy: ~4m (estimated) ✅

All targets met.
