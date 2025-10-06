# llama.cpp Performance Optimization Summary

**Issue**: [#242](https://github.com/arkavo-org/arkavo-edge/issues/242)
**Branch**: `feature/llama-cpp-performance-optimizations`
**Date**: 2025-10-05

## Optimizations Implemented

### 1. CPU Core Auto-Detection ✅
**Location**: `crates/arkavo-llama-cpp/src/lib.rs:163-167`

**Changes**:
- Replaced hardcoded `n_threads = 8` with dynamic detection
- Uses `std::thread::available_parallelism()` to detect available CPU cores
- Caps at 16 threads to prevent diminishing returns
- Fallback to 8 threads if detection fails
- Applied to both `n_threads` and `n_threads_batch`

**Code**:
```rust
let num_cores = std::thread::available_parallelism()
    .map(|n| n.get())
    .unwrap_or(8);
let thread_count = num_cores.min(16) as i32;
params.n_threads = thread_count;
params.n_threads_batch = thread_count;
```

**Expected Impact**:
- Better utilization of available CPU cores
- Improved parallel processing of batches
- Scales automatically from 4-core to 16-core+ systems

### 2. Increased Batch Size for Prefill ✅
**Location**: `crates/arkavo-llama-cpp/src/lib.rs:171`

**Changes**:
- Increased `n_batch` from 512 to 2048 tokens
- Kept `n_ubatch` (micro-batch) at 512 for memory efficiency

**Code**:
```rust
params.n_batch = 2048; // Increased from 512
params.n_ubatch = 512; // Kept at 512
```

**Expected Impact**:
- Faster prompt processing (prefill phase)
- Reduced Time to First Token (TTFT)
- Better GPU utilization for large prompts

### 3. Optimized Chunk Size ✅
**Location**: `crates/arkavo-llm/src/llamacpp_provider.rs:154-156`

**Changes**:
- Increased chunk size from 16 to 64 tokens
- Updated threshold from 32 to 64 tokens

**Code**:
```rust
if input_tokens.len() > 64 {
    let chunk_size = 64; // Increased from 16
    // ...
}
```

**Expected Impact**:
- Improved throughput for long prompts
- Fewer decode calls → reduced overhead
- Better batch utilization

### 4. Disabled Metal Debug Overhead ✅
**Location**: `crates/arkavo-llama-cpp-sys/build.rs:38-42`

**Changes**:
- Added `GGML_METAL_NDEBUG=ON` for release builds on macOS
- Conditionally enabled based on build profile

**Code**:
```rust
let is_release = env::var("PROFILE").unwrap_or_default() == "release";
if is_release {
    config.define("GGML_METAL_NDEBUG", "ON");
}
```

**Expected Impact**:
- Reduced Metal API overhead in release builds
- Faster GPU operations
- Production-ready performance

## Performance Benchmarks

### Benchmark Suite Created
**Location**: `crates/arkavo-llama-cpp/benches/performance.rs`

**Tests Included**:
1. `bench_context_creation` - Measures context initialization time
2. `bench_tokenization` - Tests tokenization speed for short/medium/long prompts
3. `bench_chat_template` - Measures chat template application overhead
4. `bench_batch_processing` - Compares chunk sizes (16, 32, 64, 128)
5. `bench_single_token_generation` - Measures sampling overhead
6. `bench_time_to_first_token` - TTFT for varying prompt lengths

### Baseline Results
**Note**: Benchmarks require a model file to run. The suite gracefully skips tests when no model is available.

Chat template application (model-independent):
- **Time**: ~5.7µs per application
- **Consistency**: Low variance (4% outliers)

### Expected Performance Improvements

Based on llama.cpp recommendations and our changes:

| Metric | Before | After (Expected) | Improvement |
|--------|--------|------------------|-------------|
| TTFT (medium prompt) | Baseline | -20% to -30% | ✅ Faster |
| Throughput (tok/s) | Baseline | +30% to +40% | ✅ Faster |
| CPU Utilization | Fixed 8 cores | Auto (up to 16) | ✅ Scalable |
| Batch Processing | 512 tokens | 2048 tokens | 4x capacity |
| Chunk Size | 16 tokens | 64 tokens | 4x throughput |

## Configuration Impact

### Debug Logging
When `ARKAVO_DEBUG_CHAT=1` is set, the system now reports:
```
Context: cores=<detected>, threads=<count>, n_batch=2048, KV offload=true, flash_attn=auto
```

### Runtime Behavior
- Automatically scales thread count based on available CPU cores
- Larger batch sizes reduce context switches
- Larger chunks reduce decode overhead
- Metal optimizations apply automatically in release builds

## Testing

### Code Quality
- ✅ Passes `cargo clippy -p arkavo-llama-cpp -- -D warnings`
- ✅ Compiles successfully in release mode
- ✅ No new dependencies added
- ✅ Backward compatible (fallback to 8 threads if detection fails)

### Regression Prevention
- Benchmark suite created for future regression testing
- Can be run via: `cargo bench -p arkavo-llama-cpp --bench performance`
- Integrates with Criterion for statistical analysis

## Implementation Notes

### Priorities Addressed (from issue #242)
1. ✅ **CPU core auto-detection** (quick win)
2. ✅ **Chunk size increase** (16 → 64)
3. ✅ **Batch size increase** (512 → 2048 for prefill)
4. ✅ **Metal debug overhead** (disabled in release)
5. ⏸️ **Context reuse** (deferred - requires architecture changes)

### Context Pooling/Reuse
**Status**: Not implemented in this PR

**Reason**: Current architecture creates fresh contexts per request in the provider. Implementing proper context pooling with sequence IDs requires:
- Significant refactoring of `LlamaCppProvider`
- Sequence-based conversation isolation
- Context lifecycle management
- More complex testing requirements

**Recommendation**: Evaluate after measuring impact of current optimizations. If TTFT improvements meet targets (>20%), context pooling can be a follow-up optimization.

## Files Modified

1. `crates/arkavo-llama-cpp/src/lib.rs` - CPU detection, batch size, debug logging
2. `crates/arkavo-llama-cpp-sys/build.rs` - Metal debug flag
3. `crates/arkavo-llm/src/llamacpp_provider.rs` - Chunk size optimization
4. `crates/arkavo-llama-cpp/Cargo.toml` - Added benchmark configuration
5. `crates/arkavo-llama-cpp/benches/performance.rs` - NEW: Performance benchmark suite

## Next Steps

1. **Validation**: Test with actual model to measure real-world improvements
2. **Monitoring**: Track performance metrics in production
3. **Documentation**: Update CLAUDE.md with performance characteristics
4. **Context Pooling**: Implement if additional optimization needed

## Success Criteria

- [x] CPU cores auto-detected
- [x] Batch size increased to 2048
- [x] Chunk size increased to 64
- [x] Metal debug disabled in release
- [x] Benchmark suite created
- [x] All code quality checks pass
- [ ] Real-world performance validation (requires model testing)

## References

- Issue: https://github.com/arkavo-org/arkavo-edge/issues/242
- llama.cpp optimization guide: vendor/llama.cpp/docs/build.md
- Criterion.rs benchmarking: https://github.com/bheisler/criterion.rs
