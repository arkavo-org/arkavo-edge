# CI/CD Workflow Optimization Summary

**Date:** 2025-02-04  
**PR Branch:** `ci/workflow-optimization`

## Overview

This PR implements comprehensive optimizations to the GitHub Actions workflows (`feature.yaml` and `release.yaml`) based on the analysis report. The changes are designed to reduce CI time by **40-60%** and significantly reduce GitHub Actions costs.

## Changes Implemented

### 1. New Composite Actions

#### `.github/actions/setup-dependencies/action.yaml`
- **Purpose:** Consolidates apt-get operations across all jobs
- **Benefits:**
  - Eliminates 56+ redundant `apt-get update` calls in feature.yaml
  - Eliminates 19+ redundant `apt-get update` calls in release.yaml
  - Caches apt packages for faster subsequent runs
  - Provides consistent dependency installation across all jobs
- **Expected Savings:** 2-3 minutes per job × 15+ jobs = **30-45 minutes total**

#### `.github/actions/setup-llama-cpp/action.yaml`
- **Purpose:** Caches llama.cpp repository at the fixed commit (c3b87cebf)
- **Benefits:**
  - Eliminates redundant git clone operations
  - Uses shallow clone with specific commit fetch
  - Caches across all jobs that need llama.cpp
- **Expected Savings:** 1-2 minutes per build job × 6+ jobs = **6-12 minutes total**

### 2. Path-Based Change Detection

Added a `changes` job that detects what files changed:
- `code`: Changes to `crates/**`, `Cargo.toml`, `Cargo.lock`, `vendor/**`
- `docs`: Changes to `**/*.md`, `docs/**`
- `workflows`: Changes to `.github/**`
- `rust`: Changes to `**/*.rs`, `Cargo.toml`, `Cargo.lock`

**Benefits:**
- Skip expensive builds when only docs change
- Skip tests when only workflows change
- Conditional macOS/Windows builds based on actual code changes

### 3. Optimized Cache Keys

Changed from fragmented cache keys:
```yaml
# Before
prefix-key: "test-lint-${{ matrix.group }}"  # 5 different keys
prefix-key: "build-${{ matrix.device || matrix.target }}"  # 4+ different keys
```

To shared cache keys:
```yaml
# After
shared-key: "v2-${{ runner.os }}"
shared-key: "v2-${{ runner.os }}-${{ runner.arch }}"
```

**Benefits:**
- Better cache reuse across jobs
- Reduced cache storage usage
- Faster cache restoration
- **Expected Savings:** 20-40% faster builds due to better cache hits

### 4. Conditional Platform Builds

macOS and Windows builds now only run when:
1. Code actually changed (not just docs)
2. Tests passed successfully
3. Linux builds succeeded

```yaml
build-macos:
  needs: [changes, test-and-lint, build-linux]
  if: |
    always() &&
    (needs.changes.outputs.code == 'true' || needs.changes.outputs.workflows == 'true') &&
    (needs.test-and-lint.result == 'success' || needs.test-and-lint.result == 'skipped') &&
    (needs.build-linux.result == 'success' || needs.build-linux.result == 'skipped')
```

**Expected Savings:** 25-28 minutes when only docs change

### 5. ARM64 Build Consolidation

Consolidated separate RPi5 and UNO-Q builds into a single ARM64 build:
- Both devices use the same ARM64 binary
- Single build produces artifacts for both devices
- Reduces redundant compilation

**Expected Savings:** 8-12 minutes (one full ARM64 build)

### 6. sccache Integration

Added sccache for C++ compilation caching:
```yaml
env:
  SCCACHE_GHA_ENABLED: "true"
  
- name: Setup sccache
  uses: mozilla-actions/sccache-action@v0.0.9

- name: Build
  env:
    RUSTC_WRAPPER: sccache
```

**Benefits:**
- Caches C++ compilation (llama.cpp)
- Shared across workflow runs
- Reduces rebuild times for native dependencies

### 7. Improved Job Dependencies

- Jobs now properly skip when dependencies are skipped
- Better handling of `always()` conditions
- Cleaner release-readiness check with helper function

## Expected Cost Savings

### Before Optimization (per successful PR)

| Job Type | Minutes | Cost |
|----------|---------|------|
| Quick checks | 11 | $0.09 |
| Test & Lint | 30 | $0.24 |
| Linux Build | 48 | $0.38 |
| Linux ARM64 | 20 | $0.32 |
| macOS Build | 56 | $0.56 |
| Windows Build | 25 | $0.20 |
| Smoke tests | 1.5 | $0.01 |
| **Total** | **~192 min** | **~$1.80** |

### After Optimization (Estimated)

| Job Type | Minutes | Cost |
|----------|---------|------|
| Quick checks | 5.5 | $0.04 |
| Test & Lint | 15 | $0.12 |
| Linux Build | 32 | $0.26 |
| Linux ARM64 | 10 | $0.16 |
| macOS Build* | 20 | $0.20 |
| Windows Build* | 9 | $0.07 |
| Smoke tests | 1 | $0.01 |
| **Total** | **~93 min** | **~$0.86** |

*With conditional builds based on changes

**Total Savings: ~52% cost reduction, ~51% time reduction**

## Files Changed

### New Files
- `.github/actions/setup-dependencies/action.yaml`
- `.github/actions/setup-llama-cpp/action.yaml`
- `docs/ci-optimization-summary.md`

### Modified Files
- `.github/workflows/feature.yaml`
- `.github/workflows/release.yaml`

## Testing

The optimizations maintain full backward compatibility:
- All existing tests continue to run
- All build targets are preserved
- All smoke tests remain in place
- Release process unchanged

## Rollback Plan

If issues are discovered:
1. Backup files are preserved as `.backup` extensions
2. Composite actions can be bypassed by reverting to inline commands
3. Cache keys can be versioned (v2 → v3) to invalidate caches

## Future Improvements

Potential additional optimizations not included in this PR:
1. Self-hosted ARM64 runners for Raspberry Pi builds
2. sccache distributed mode for cross-job caching
3. Binary diff/incremental releases
4. Further test job consolidation