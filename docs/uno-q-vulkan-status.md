# UNO Q Vulkan Status Report

## Current State

**Build**: ✅ Successfully compiling with Vulkan support
**Device Detection**: ✅ Adreno 702 detected via Turnip driver
**Initialization**: ✅ Vulkan device and context creation succeed
**Inference**: ❌ Device loss during execution (`vk::DeviceLostError`)

## What We've Tried

### 1. FP32 Fallback Patch (Not Applicable)
**Assumption**: Turnip lacks 16-bit storage support
**Reality**: `storageBuffer16BitAccess = true` ✅
**Result**: Patch code path never executes

### 2. Adreno Batch Size Limits
**Implementation**: Added `QUALCOMM_ADRENO` architecture detection
- Set `n_batch = 16`, `n_ubatch = 16` (below 33 limit)
- Reduced context to 2048 tokens
**Result**: Still crashes with `ErrorDeviceLost`

### 3. Reduced GPU Layers
**Test**: Limited offloading to 10 layers
**Result**: Still crashes immediately

## Technical Details

### Vulkan Environment
- **Driver**: Mesa Turnip 25.1.0-1qcom1
- **API**: Vulkan 1.0.311
- **GPU**: Turnip Adreno (TM) 702 (vendorID 0x5143)
- **Features**:
  - `storageBuffer16BitAccess = true`
  - `uniformAndStorageBuffer16BitAccess = false`

### Crash Pattern
```
llama_context: n_ctx_per_seq (2048) < n_ctx_train (32768) -- the full capacity of the model will not be utilized
terminate called after throwing an instance of 'vk::DeviceLostError'
  what():  vk::Device::waitForFences: ErrorDeviceLost
Aborted
```

- Crash happens during `waitForFences()` call
- Occurs very early (likely during context/buffer setup)
- Not related to model size or layer count

## Root Cause Hypotheses

### Most Likely
1. **Shader Compilation Failure**
   - Adreno has known shader compiler bugs
   - llama.cpp shaders may not be compatible with Turnip
   - Need validation layers to see actual error

2. **Memory Allocation Pattern**
   - llama.cpp may allocate buffers exceeding 1GB limit
   - Turnip might not handle allocation failures gracefully
   - Need `GGML_VULKAN_MEMORY_DEBUG=1`

3. **Command Buffer Submission**
   - Research shows Adreno prefers one-by-one submission
   - llama.cpp may batch commands incompatibly
   - Could require llama.cpp code changes

### Less Likely
4. **Missing Vulkan Extensions** - All required extensions present
5. **Driver Version** - Mesa 25.1.0 is recent
6. **Batch Size** - Already reduced to 16

## Implemented Patches

### llama.cpp (vendor/llama.cpp)
1. FP32 fallback for missing 16-bit storage (lines 4152-4157)
   - Not triggered on Turnip
2. Adreno architecture detection (line 342-345)
   - Working correctly

### Rust Wrapper (crates/arkavo-llama-cpp)
1. Adreno batch size limits (lines 174-184)
   - `n_batch = 16`, `n_ubatch = 16`
   - Context = 2048

## Validation Run Results

### Test Command
```bash
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/freedreno_icd.json \
VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation \
VK_LOADER_DEBUG=all \
GGML_VULKAN_DEBUG=1 \
GGML_VULKAN_MEMORY_DEBUG=1 \
./arkavo-adreno chat --model gemma-3-270m-it-Q4_0.gguf --prompt 'Test' --max-tokens 5
```

### Key Findings

**1. Validation Layer Not Available**
```
[Vulkan Loader] ERROR | LAYER: Layer "VK_LAYER_KHRONOS_validation" was not found
```
- VK_LAYER_KHRONOS_validation not installed on UNO Q
- Only available layer: `VK_LAYER_MESA_overlay` and `VK_LAYER_MESA_device_select`

**2. GGML Debug Flags Ineffective**
- `GGML_VULKAN_DEBUG=1` produced no output
- `GGML_VULKAN_MEMORY_DEBUG=1` produced no output
- Suggests either:
  - Not compiled with debug support
  - Crash occurs before debug logging starts
  - Environment variables not read correctly

**3. Vulkan Initialization Succeeds**
```
[Vulkan Loader] DRIVER: Using "Turnip Adreno (TM) 702" with driver: "libvulkan_freedreno.so"
```
- Instance creation: ✅
- Physical device enumeration: ✅
- Logical device creation: ✅
- Queue creation: ✅

**4. Crash Location**
```
llama_context: n_ctx_per_seq (2048) < n_ctx_train (32768) -- the full capacity of the model will not be utilized
terminate called after throwing an instance of 'vk::DeviceLostError'
  what():  vk::Device::waitForFences: ErrorDeviceLost
```
- Context created successfully
- Crash during first GPU operation (`waitForFences`)
- No llama.cpp Vulkan debug output before crash

**5. Mesa Layer Warning (Non-Fatal)**
```
[Vulkan Loader] INFO | LAYER: Failed to find vkGetDeviceProcAddr in layer "libVkLayer_MESA_device_select.so"
```
- Known Mesa layer limitation
- Not related to crash

## Next Steps to Debug

### 1. Install Validation Layers on Device
```bash
# On UNO Q (if possible):
apt install vulkan-validationlayers
```

### 2. Enable Turnip Debug
```bash
# Mesa Turnip-specific debug
export MESA_DEBUG=1
export TU_DEBUG=startup,ir3,gmem
export NIR_PRINT=1
```

### 3. Try Alternative Approaches
1. **Update llama.cpp** - We're 175 commits behind, may have Adreno fixes
2. **Rebuild with debug** - Ensure `GGML_VULKAN_DEBUG` is compiled in
3. **Test minimal operation** - Simple Vulkan compute shader test
4. **OpenCL backend** - Qualcomm's official recommendation for Adreno
5. **Upstream bug report** - llama.cpp issue with detailed Turnip logs

## Files Modified
- `vendor/llama.cpp/ggml/src/ggml-vulkan/ggml-vulkan.cpp`
- `crates/arkavo-llama-cpp/src/lib.rs`
- `.github/workflows/feature.yaml`
- `patches/llama.cpp/turnip-fp32-fallback.patch`
- `docs/uno-q-vulkan-debug.md`

## Update: llama.cpp 5d195f17bc (Oct 25, 2025)

### Attempted Fix
Updated llama.cpp from e95fec640f (Oct 1) to 5d195f17bc (Oct 25), bringing:
- 176 commits of upstream improvements
- Vulkan memory allocation fix (#16354) for VK_WHOLE_SIZE handling
- Multiple other Vulkan improvements

### Test Result
**Status**: ❌ Still crashes with identical error

```bash
$ VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/freedreno_icd.json \
  GGML_VULKAN_DEBUG=1 GGML_VULKAN_MEMORY_DEBUG=1 \
  ./arkavo-adreno chat --model gemma-3-270m-it-Q4_0.gguf --prompt 'Test' --max-tokens 5

llama_context: n_ctx_per_seq (2048) < n_ctx_train (32768) -- the full capacity of the model will not be utilized
terminate called after throwing an instance of 'vk::DeviceLostError'
  what():  vk::Device::waitForFences: ErrorDeviceLost
Aborted
```

The memory allocation fix (#16354) addresses >4GB support, not the Adreno 1GB limit issue.

### OpenCL Investigation

Investigated enabling OpenCL backend as alternative:
- ❌ Mesa's Rusticl and Clover report 0 OpenCL devices
- ❌ Error: `unsupported GPU id 0x0 / chip id 0xb207000200` (Adreno 702)
- ❌ Mesa OpenCL implementations don't support Qualcomm Adreno GPUs
- ℹ️ llama.cpp's "Adreno optimizations" require Qualcomm's proprietary OpenCL driver (not installed)

See `docs/uno-q-opencl-investigation.md` for full details.

## Conclusion

Vulkan on Adreno 702 via Turnip initializes successfully but crashes during execution. The issue is likely shader compilation bugs, memory allocation exceeding 1GB limit, or command buffer submission patterns incompatible with Turnip.

Based on upstream llama.cpp Issue #5186:
- This was identified as a **Mesa Turnip driver limitation**, not fixable in application code
- Even with workarounds, performance was only ~0.5 tokens/second (unusable)
- Issue was closed as "driver bug" rather than application bug

### Recommendations

1. **CPU-only mode** - Most practical option for UNO Q with current software stack
2. **Wait for Mesa Turnip improvements** - Track Mesa development for Adreno support
3. **Qualcomm proprietary OpenCL** - Would require vendor SDK installation
4. **Alternate models** - Try CPU-optimized quantizations (Q4_K_M, Q5_K_M)

**Status**: Vulkan GPU acceleration not viable on UNO Q with Mesa Turnip at this time.

## Complete Hardware Acceleration Investigation

See `docs/uno-q-hardware-acceleration-blockers.md` for comprehensive analysis of all acceleration paths:
- Vulkan (GPU) - Driver bugs
- OpenCL (GPU) - Not supported by Mesa
- Hexagon NPU - Missing device tree configuration

All paths are blocked by either driver limitations or build constraints (no Docker, OOM on native builds).
