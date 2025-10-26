# UNO Q Vulkan Device Loss Debugging

## Problem Summary

Initial Vulkan implementation on UNO Q (Adreno 702 GPU with Mesa Turnip driver) crashed during inference with `vk::Device::waitForFences: ErrorDeviceLost`. Investigation revealed the root cause was an incompatibility between Mesa Turnip and llama.cpp's Vulkan backend requirements.

## Root Cause Analysis

### Mesa Turnip 16-bit Storage Limitation

Mesa Turnip (the open-source Vulkan driver for Qualcomm Adreno GPUs) reports `storageBuffer16BitAccess = false`, indicating lack of 16-bit floating point storage support. This is a known limitation of the current Turnip implementation.

### llama.cpp Hard Requirement

llama.cpp's Vulkan backend (`ggml-vulkan.cpp:4151-4154`) had a hard requirement for 16-bit storage:

```cpp
if (!vk11_features.storageBuffer16BitAccess) {
    std::cerr << "ggml_vulkan: device does not support 16-bit storage." << std::endl;
    throw std::runtime_error("Unsupported device");
}
```

This caused the device initialization to fail with "Unsupported device" error before inference could even start.

### Upstream Issue Status

GitHub Issue [#7620](https://github.com/ggml-org/llama.cpp/issues/7620) requested support for Vulkan devices without 16-bit storage but was closed without implementation. The maintainers noted that "16-bit floats are part of the quantization structures," but acknowledged that FP32 fallback is technically feasible.

The community recommendation is to use OpenCL instead of Vulkan for Qualcomm Adreno GPUs, but Vulkan support is achievable with appropriate patches.

## Solution: Local Patch for FP32 Fallback

### Patch Details

**Patch File**: `docs/llama-cpp-turnip-fp32-fallback.patch`

Modified `vendor/llama.cpp/ggml/src/ggml-vulkan/ggml-vulkan.cpp:4151-4157` to:

1. Convert hard error to warning
2. Disable FP16 features when 16-bit storage is unavailable
3. Fall back to FP32 operations
4. Conditionally add `VK_KHR_16bit_storage` extension only when supported

```cpp
if (!vk11_features.storageBuffer16BitAccess) {
    std::cerr << "ggml_vulkan: device does not support 16-bit storage, falling back to FP32." << std::endl;
    device->fp16 = false;
    fp16_storage = false;
} else {
    device_extensions.push_back("VK_KHR_16bit_storage");
}
```

### Expected Impact

This patch allows llama.cpp to:
- Initialize Vulkan device successfully on Turnip
- Use FP32 operations instead of FP16
- Avoid the device loss error during initialization

**Performance Trade-offs:**
- Increased memory usage (FP32 vs FP16)
- Potentially slower inference (32-bit vs 16-bit operations)
- Still GPU-accelerated (better than CPU fallback)

## Additional Considerations

### Adreno GPU Limitations (from upstream research)

1. **Batch Size Constraint**: Device loss occurs with batch size ≥ 33
   - Workaround: Keep batch size < 32

2. **Memory Limit**: Maximum allocated memory ~1GB
   - llama.cpp v175+ includes fixes for memory allocation (#16354)

3. **Shader Compilation**: Adreno GPUs have subtle shader compiler bugs
   - Some shaders need code duplication in if-branches as workaround

### Recommendations for Testing

Test with reduced parameters to stay within Adreno constraints:
- Batch size: 16 or less
- Context window: 512-2048 tokens
- Model: Gemma-3-270M Q4_0 (small quantized model)
- Threads: Match CPU core count

### Environment Variables for Debugging

```bash
# Already handled by patch, but can force disable FP16 explicitly:
export GGML_VK_DISABLE_F16=1

# Enable llama.cpp debug logging:
export GGML_VULKAN_DEBUG=1

# Arkavo debug mode:
export ARKAVO_DEBUG=1
```

## Next Steps

1. Rebuild with the patch applied
2. Test basic inference on UNO Q device
3. If successful, tune batch size and context window for optimal performance
4. Consider updating llama.cpp to latest (we're 175 commits behind) for additional Vulkan improvements
5. Monitor memory usage and adjust model size accordingly

## Reapplying the Patch

If you update llama.cpp to a newer version, you'll need to reapply this patch:

```bash
cd vendor/llama.cpp
git apply ../../docs/llama-cpp-turnip-fp32-fallback.patch
```

If the patch fails due to code changes, manually apply the changes:
1. Locate the `storageBuffer16BitAccess` check in `ggml/src/ggml-vulkan/ggml-vulkan.cpp`
2. Replace the hard error with the FP32 fallback logic shown above
3. Regenerate the patch: `git diff HEAD > ../../docs/llama-cpp-turnip-fp32-fallback.patch`

## References

- llama.cpp Issue #7620: Support Vulkan devices without 16-bit storage
- llama.cpp Issue #8743: Adreno device failures with large batch sizes
- llama.cpp Issue #10406: Vulkan failures with quantized models on Android Termux
- Qualcomm Blog: OpenCL backend for Adreno GPUs (Nov 2024)
- Mesa Turnip PR: FP16 storage support (in progress)
