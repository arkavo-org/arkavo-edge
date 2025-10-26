# UNO Q OpenCL Investigation - Why OpenCL is Unavailable

## Summary

**OpenCL is not available for Adreno GPUs on Arduino UNO Q.** Mesa's open-source OpenCL implementations (Rusticl and Clover) do not support Qualcomm Adreno GPUs. The llama.cpp "Adreno optimizations" require Qualcomm's proprietary OpenCL driver, which is not installed on this device.

## Investigation Details

### OpenCL Libraries Present

The device has OpenCL libraries installed:

```bash
$ adb shell "ldconfig -p | grep -i opencl"
libRusticlOpenCL.so.1 (libc6,AArch64) => /lib/aarch64-linux-gnu/libRusticlOpenCL.so.1
libRusticlOpenCL.so (libc6,AArch64) => /lib/aarch64-linux-gnu/libRusticlOpenCL.so
libOpenCL.so.1 (libc6,AArch64) => /lib/aarch64-linux-gnu/libOpenCL.so.1
libMesaOpenCL.so.1 (libc6,AArch64) => /lib/aarch64-linux-gnu/libMesaOpenCL.so.1
libMesaOpenCL.so (libc6,AArch64) => /lib/aarch64-linux-gnu/libMesaOpenCL.so
```

### OpenCL Device Detection Results

```bash
$ adb shell "clinfo -l"
Platform #0: rusticl
Platform #1: Clover
MESA: error: fd_pipe_new2:49: unsupported GPU id 0x0 / chip id 0xb207000200

$ adb shell "clinfo"
Number of platforms                               2
  Platform Name                                   rusticl
  Platform Name                                   Clover

  Platform Name                                   rusticl
Number of devices                                 0

  Platform Name                                   Clover
Number of devices                                 0
```

**Key Finding**: Both OpenCL platforms report **0 devices** and error: `unsupported GPU id 0x0 / chip id 0xb207000200` (Adreno 702).

## Why OpenCL Doesn't Work

### Mesa OpenCL Implementations

1. **Rusticl** (Rust-based OpenCL on Mesa)
   - Only supports AMD RadeonSI and Intel Iris drivers
   - No support for Freedreno (Adreno) driver
   - Adreno 702 chip ID `0xb207000200` is unrecognized

2. **Clover** (Gallium3D OpenCL)
   - Mesa's older OpenCL implementation
   - Only supports AMD RadeonSI, Intel Iris, and NVIDIA Nouveau
   - No support for Freedreno (Adreno) driver

### Qualcomm's Proprietary OpenCL

llama.cpp's "Adreno optimizations" (`GGML_OPENCL_USE_ADRENO_KERNELS`) are designed for **Qualcomm's proprietary OpenCL driver**, which:
- Is available in Snapdragon development kits
- Requires vendor-specific installation
- Is **not included** in standard Debian/Ubuntu ARM64 images
- Is **not installed** on Arduino UNO Q

## Attempted Solution (Reverted)

We initially tried enabling OpenCL with Adreno optimizations:

```rust
// crates/arkavo-llama-cpp-sys/build.rs (REVERTED)
config.define("GGML_OPENCL", "ON");
config.define("GGML_OPENCL_USE_ADRENO_KERNELS", "ON");
println!("cargo:rustc-link-lib=OpenCL");
```

**Result**: Would compile but runtime would have 0 OpenCL devices available, falling back to CPU-only mode.

## Current Strategy: Vulkan-Only

Since OpenCL is unavailable, we're using **Vulkan via Mesa Turnip** as the only GPU backend:

```rust
// crates/arkavo-llama-cpp-sys/build.rs (CURRENT)
config.define("GGML_VULKAN", "ON");
config.define("GGML_OPENCL", "OFF"); // Mesa OpenCL doesn't support Adreno
```

### Vulkan Status

- ✅ Adreno 702 detected via Turnip driver
- ✅ Vulkan device and context creation succeed
- ❌ Device loss during inference (`vk::DeviceLostError`)

## Open Questions

1. **Could Qualcomm's OpenCL driver be installed?**
   - Requires vendor-specific packages
   - May not be compatible with Mesa Turnip
   - Would need investigation with Qualcomm SDKs

2. **Why does Vulkan crash?**
   - Current hypothesis: Shader compilation bugs, memory allocation issues, or command buffer submission patterns
   - Updated llama.cpp (Oct 25) includes memory allocation fixes
   - Testing needed to verify if fixes resolve device loss

## Related Files

- `docs/uno-q-vulkan-status.md` - Vulkan debugging status
- `docs/uno-q-vulkan-debug.md` - Detailed Vulkan investigation
- `crates/arkavo-llama-cpp-sys/build.rs` - Build configuration
- `patches/llama.cpp/turnip-fp32-fallback.patch` - Vulkan FP32 fallback patch

## References

- Mesa Rusticl: https://docs.mesa3d.org/rusticl.html
  - Supported: RadeonSI, Iris
  - Not supported: Freedreno (Adreno)
- Mesa Clover: https://docs.mesa3d.org/gallium/drivers/freedreno.html
  - Freedreno driver has no OpenCL support
- llama.cpp Adreno OpenCL: https://github.com/ggerganov/llama.cpp/pull/10693
  - Requires Qualcomm's proprietary OpenCL driver
  - Blog: https://www.qualcomm.com/developer/blog/2024/11/introducing-new-opn-cl-gpu-backend-llama-cpp-for-qualcomm-adreno-gpu

## Conclusion

OpenCL is not a viable option for Adreno GPU acceleration on UNO Q without Qualcomm's proprietary driver. Focus remains on resolving Vulkan device loss issues with the updated llama.cpp codebase.

---

**Last Updated**: 2025-10-25
**Status**: Documented - OpenCL avenue closed, proceeding with Vulkan-only approach
