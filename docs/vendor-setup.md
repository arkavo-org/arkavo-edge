# Vendor Dependencies Setup

This document describes how to set up vendored dependencies that are not included in the git repository.

## llama.cpp

The `vendor/llama.cpp` directory contains the llama.cpp C++ library for local LLM inference. This is not tracked in git to avoid repository bloat.

### Initial Setup

Clone the llama.cpp repository and apply arkavo-specific patches:

```bash
# Clone llama.cpp to the vendor directory
git clone https://github.com/ggerganov/llama.cpp vendor/llama.cpp

# Apply arkavo-specific patches
cd vendor/llama.cpp
git apply ../../patches/llama.cpp/turnip-fp32-fallback.patch
```

### Updating llama.cpp

To update to a newer version of llama.cpp:

```bash
cd vendor/llama.cpp

# Fetch latest changes
git fetch origin

# Checkout desired version (use commit hash, tag, or branch)
git checkout <version>

# Reapply arkavo patches
git apply ../../patches/llama.cpp/turnip-fp32-fallback.patch
```

Note: After updating, you may need to resolve conflicts if the patch no longer applies cleanly. Update the patch file in `patches/llama.cpp/` if necessary.

### Patches

#### turnip-fp32-fallback.patch

Adds compatibility for Qualcomm Adreno GPUs running with Mesa Turnip driver:

- **FP32 Fallback**: Gracefully falls back to FP32 when GPU doesn't support 16-bit storage (required for Adreno 702)
- **Adreno Detection**: Adds Qualcomm vendor ID and Adreno architecture detection to Vulkan backend
- **Device-specific Optimizations**: Enables future device-specific tuning for Adreno GPUs

This patch is required for UNO Q (Qualcomm Snapdragon 782G with Adreno 702 GPU) support.

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

**Patch fails to apply**:
- The llama.cpp version may have changed significantly
- Manually resolve conflicts or update the patch file
- See `git apply --3way` for easier conflict resolution

**GPU acceleration not working**:
- Check that the patch was applied correctly: `cd vendor/llama.cpp && git log -1`
- Verify build configuration in `crates/arkavo-llama-cpp-sys/build.rs`
- For UNO Q: ensure `ARKAVO_TARGET_DEVICE=uno-q` is set during build
