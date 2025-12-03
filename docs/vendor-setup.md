# Vendor Dependencies Setup

This document describes how to set up vendored dependencies that are not included in the git repository.

## llama.cpp

The `vendor/llama.cpp` directory contains the llama.cpp C++ library for local LLM inference. This is not tracked in git to avoid repository bloat.

### Initial Setup

Clone the llama.cpp repository:

```bash
# Clone llama.cpp to the vendor directory
git clone https://github.com/ggerganov/llama.cpp vendor/llama.cpp

# Checkout the required version (includes Ministral 3 support)
cd vendor/llama.cpp
git checkout ecf74a841  # Ministral 3 + mtmd warmup field
```

### Required Version

**Recommended commit:** `ecf74a841` (mtmd: add warmup option) - December 1, 2025

This commit includes:
- `cd3c11890`: Ministral 3 architecture support (3B/8B/14B)
- `ecf74a841`: mtmd_context_params::warmup field for multimodal

### Updating llama.cpp

To update to a newer version of llama.cpp:

```bash
cd vendor/llama.cpp

# Fetch latest changes
git fetch origin

# Checkout desired version (use commit hash, tag, or branch)
git checkout <version>
```

After updating, rebuild the project to regenerate FFI bindings.

### Build Integration

The `arkavo-llama-cpp-sys` crate's build script (`crates/arkavo-llama-cpp-sys/build.rs`) expects to find llama.cpp at `vendor/llama.cpp` and will:

1. Build llama.cpp using CMake
2. Link the static libraries into the Rust binary
3. Apply appropriate GPU acceleration settings based on the target platform:
   - **macOS**: Metal GPU acceleration
   - **Linux ARM64 (UNO Q)**: Vulkan GPU acceleration with Adreno optimizations
   - **Linux ARM64 (Raspberry Pi)**: CPU-only with OpenMP
   - **Linux x86_64**: CPU-only with OpenMP
   - **Windows**: CPU-only (planned: Vulkan support)

### Troubleshooting

**Build fails with "llama.h not found"**:
- Ensure `vendor/llama.cpp` exists and contains the llama.cpp source code
- Run the setup commands above

**FFI errors after llama.cpp update**:
- The llama.cpp API may have changed
- Check for struct field changes in `crates/arkavo-llama-cpp/src/multimodal.rs`
- Rebuild with `cargo clean -p arkavo-llama-cpp-sys && cargo build`

**GPU acceleration not working**:
- Verify build configuration in `crates/arkavo-llama-cpp-sys/build.rs`
- Check llama.cpp version: `cd vendor/llama.cpp && git log -1`
- For UNO Q: ensure `ARKAVO_TARGET_DEVICE=uno-q` is set during build
