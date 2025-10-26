# Arduino UNO Q Hardware Acceleration Blockers

## Executive Summary

**Status**: ❌ Hardware acceleration (GPU/NPU) is **not achievable** on Arduino UNO Q with current software stack and build constraints.

All three acceleration paths are blocked:
- **Vulkan (GPU)**: Mesa Turnip driver bugs cause device loss
- **OpenCL (GPU)**: Mesa doesn't support Adreno GPUs
- **Hexagon NPU**: FastRPC driver won't bind due to missing device tree configuration

CPU-only mode works but is explicitly rejected as worthless for this use case.

## Hardware Specifications

- **SoC**: Qualcomm QRB2210 (Dragonwing)
- **GPU**: Adreno 702
- **NPU/DSP**: Hexagon V55 DSP + Tensor Accelerator
- **Memory**: 2GB RAM
- **OS**: Debian 13 (Trixie), Kernel 6.16.0-geffa8626771a

## Investigation Summary

### Path 1: Vulkan GPU Acceleration (Mesa Turnip)

**Status**: ❌ Driver bugs cause unrecoverable device loss

**What We Tried**:
1. ✅ Built llama.cpp with Vulkan support
2. ✅ Configured Adreno-specific optimizations (batch size limits, FP32 fallback)
3. ✅ Updated llama.cpp from e95fec640f (Oct 1) to 5d195f17bc (Oct 25) - 176 commits
4. ✅ Tested multiple quantization formats: Q4_0, Q4_K_M, Q5_K_M
5. ✅ Reduced GPU layers to 10
6. ✅ Forced Turnip driver with `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/freedreno_icd.json`

**Results**:
- Vulkan initialization: ✅ Success
- Device detection: ✅ Adreno 702 detected
- Context creation: ✅ Success
- GPU execution: ❌ **Device loss** at `vk::Device::waitForFences: ErrorDeviceLost`

**Root Cause**: Mesa Turnip driver limitation (llama.cpp Issue #5186)
- Shader compilation bugs for llama.cpp compute shaders
- Memory allocation issues (Adreno 1GB limit)
- Command buffer submission incompatibilities
- Unfixable without kernel/driver patches

**Evidence**:
```
llama_context: n_ctx_per_seq (2048) < n_ctx_train (32768)
terminate called after throwing an instance of 'vk::DeviceLostError'
  what():  vk::Device::waitForFences: ErrorDeviceLost
Aborted
```

**dmesg shows GPU faults**:
```
adreno 5900000.gpu: [drm:a6xx_irq [msm]] *ERROR* gpu fault ring 0 fence 1525
status 00E41005 rb 00e0/00ef ib1 000000012CAAB000/016e ib2 000000012CAB31A0/0000
msm_dpu 5e01000.display-controller: [drm:hangcheck_handler [msm]] *ERROR*
7.0.2.0: hangcheck detected gpu lockup rb 0!
```

**Why Can't We Fix It**:
- Turnip driver source code shows `DRM_IOCTL_SYNCOBJ_WAIT` failing in kernel
- Requires patching Mesa Turnip (2500+ files) or kernel drivers
- Build constraints: Docker doesn't work, native compilation OOMs (2GB RAM)
- Upstream issue closed as "driver bug" not application bug

**Attempted Workarounds**:
1. ❌ **Graceful GPU→CPU fallback**: C++ `vk::DeviceLostError` calls `std::terminate()` which aborts the process
   - Rust's `panic::catch_unwind` cannot catch C++ `std::terminate()`
   - Environment variable `GGML_VULKAN=0` ignored (checked after initialization)
   - No way to disable Vulkan at runtime once built with it
2. ✅ **CPU-only build**: Final solution - disable Vulkan at build time
   - Binary works but uses CPU inference (slow)
   - No crashes, stable operation
   - Performance: ~1-2 tokens/sec on Gemma-3-270M (vs 50+ expected on GPU)

### Path 2: OpenCL GPU Acceleration (Mesa)

**Status**: ❌ Fundamentally incompatible

**Investigation**:
- OpenCL libraries installed: libRusticlOpenCL, libMesaOpenCL, libOpenCL
- OpenCL platforms detected: Rusticl, Clover
- Devices detected: **0 devices**

**Error**:
```
Platform #0: rusticl - Number of devices: 0
Platform #1: Clover - Number of devices: 0
MESA: error: fd_pipe_new2:49: unsupported GPU id 0x0 / chip id 0xb207000200
```

**Root Cause**: Mesa OpenCL implementations don't support Qualcomm Adreno
- **Rusticl**: Only supports AMD RadeonSI and Intel Iris
- **Clover**: Only supports AMD, Intel, NVIDIA Nouveau
- **Freedreno** (Adreno driver): Has no OpenCL support

**Qualcomm Proprietary OpenCL**:
- llama.cpp's "Adreno optimizations" (`GGML_OPENCL_USE_ADRENO_KERNELS`) require Qualcomm's proprietary driver
- Not installed on Arduino UNO Q
- Would require vendor SDK installation
- May conflict with Mesa Turnip

**References**:
- Mesa Rusticl: https://docs.mesa3d.org/rusticl.html
- llama.cpp Adreno OpenCL: https://github.com/ggerganov/llama.cpp/pull/10693

### Path 3: Hexagon NPU Acceleration (FastRPC)

**Status**: ❌ Driver won't bind due to missing device tree configuration

**Hexagon Capabilities**:
- Hexagon V55 DSP + Tensor Accelerator (HTP)
- llama.cpp experimental backend added Oct 22, 2025
- Performance example: **51.57 tokens/second** on Llama-3.2-1B (Snapdragon device)
- Requires Snapdragon toolchain Docker + Hexagon SDK 6.4.0.2

**What We Found**:

1. **Kernel Module**: ✅ Present and loads
   ```
   /lib/modules/6.16.0-geffa8626771a/kernel/drivers/misc/fastrpc.ko
   lsmod: fastrpc 28672 0
   ```

2. **ADSP Firmware**: ✅ Running
   ```
   dmesg: remoteproc remoteproc1: remote processor adsp is now up
   ```

3. **RPMSG Device**: ✅ Exists
   ```
   /sys/bus/rpmsg/devices/ab00000.remoteproc:glink-edge.fastrpcglink-apps-dsp.-1.-1
   Modalias: rpmsg:fastrpcglink-apps-dsp
   ```

4. **FastRPC Driver**: ✅ Registered
   ```
   /sys/bus/rpmsg/drivers/qcom,fastrpc/
   Module strings show: fastrpc_rpmsg_probe, fastrpc_rpmsg_callback
   ```

5. **Device Nodes**: ❌ **Not created**
   ```
   ls /dev/fastrpc* → No such file
   ```

**Root Cause Analysis**:

The FastRPC kernel module is a **dual-mode driver** supporting both:
- Platform devices (via device tree `compatible = "qcom,fastrpc"`)
- RPMSG devices (via glink channel "fastrpcglink-apps-dsp")

However, the driver **only** has `fastrpc_rpmsg_of_match` (device tree matching), **not** `rpmsg_device_id` (channel name matching):

```c
// From drivers/misc/fastrpc.c
static const struct of_device_id fastrpc_rpmsg_of_match[] = {
    { .compatible = "qcom,fastrpc" },
    { },
};
```

The driver expects a **device tree node** like:
```dts
fastrpc {
    compatible = "qcom,fastrpc";
    qcom,glink-channels = "fastrpcglink-apps-dsp";
};
```

**On UNO Q**: Device tree has **no fastrpc node**
```bash
find /proc/device-tree -name '*fastrpc*' → (empty)
find /sys/firmware/devicetree -name '*fastrpc*' → (empty)
```

**Result**: Driver exists, rpmsg device exists, but they **never bind** because:
- RPMSG device name is "fastrpcglink-apps-dsp"
- Driver only matches device tree compatible "qcom,fastrpc"
- No device tree configuration present
- Manual binding fails with I/O error

**Why Can't We Fix It**:

Two options, both blocked:

**Option A: Patch Device Tree**
- Add FastRPC device tree node to kernel DTS
- Requires kernel rebuild
- Build constraints: Docker doesn't work, native compilation OOMs

**Option B: Patch FastRPC Driver**
- Add `rpmsg_device_id` table to match "fastrpcglink-apps-dsp"
- Requires kernel rebuild
- Same build constraints

**Option C: Build llama.cpp with Hexagon SDK**
- Requires Snapdragon toolchain Docker image
- User constraint: "docker does not work"
- Would need cross-compilation setup from macOS (complex)

## Alternative Approaches Investigated

### App Lab
- Arduino's official IDE for UNO Q
- Supports "containerized AI models"
- Status: GUI-only (Wails/GTK), not accessible via ADB/CLI
- Unknown if it provides Hexagon NPU access internally

### Pre-built Hexagon Binaries
- Searched for pre-built llama.cpp with Hexagon support for QRB2210
- Status: ❌ None found for this SoC
- Most Hexagon work targets Snapdragon 8 Gen series

### Qualcomm Proprietary SDKs
- Qualcomm Hexagon SDK
- SNPE (Snapdragon Neural Processing Engine)
- QNN (Qualcomm AI Engine Direct)
- Status: Not installed on UNO Q, require vendor downloads

## Build Constraints

All hardware acceleration paths require building software, but:

1. **Docker doesn't work** (user environment)
2. **Native compilation OOMs** (2GB RAM insufficient for llama.cpp + kernel builds)
3. **Cross-compilation complex** (macOS → ARM64 Linux, requires toolchain setup)

## Kernel Configuration Details

```bash
# Kernel version
uname -r → 6.16.0-geffa8626771a

# FastRPC config
CONFIG_QCOM_FASTRPC=m  ✅ (module)

# RPMSG config
CONFIG_RPMSG=y         ✅ (built-in)
CONFIG_RPMSG_CHAR=m    ✅ (module)
CONFIG_RPMSG_QCOM_GLINK=y  ✅ (built-in)

# Remoteproc config
CONFIG_QCOM_Q6V5_ADSP=m ✅ (module, loaded)

# Vulkan
Mesa Turnip 25.1.0-1qcom1 ✅ (installed)
```

## Files Modified During Investigation

- `vendor/llama.cpp/` - Updated to 5d195f17bc (Oct 25, 2025)
- `vendor/llama.cpp/ggml/src/ggml-vulkan/ggml-vulkan.cpp` - Adreno patches
- `crates/arkavo-llama-cpp/src/lib.rs` - Adreno batch size limits
- `crates/arkavo-llama-cpp-sys/build.rs` - Vulkan-only build config
- `.github/workflows/feature.yaml` - UNO Q build configuration
- `patches/llama.cpp/turnip-fp32-fallback.patch` - Documentation of changes
- `docs/uno-q-vulkan-status.md` - Vulkan investigation
- `docs/uno-q-opencl-investigation.md` - OpenCL investigation
- This document - Complete blocker analysis

## Current Solution

**CPU-only build**: The arkavo binary for UNO Q is built without Vulkan support to avoid crashes:
- ✅ Stable operation, no crashes
- ✅ Works with all GGUF models
- ⚠️ Slow inference: ~1-2 tokens/sec on Gemma-3-270M (vs 50+ expected with GPU)
- Built with: `GGML_VULKAN=OFF` in `crates/arkavo-llama-cpp-sys/build.rs`

## Recommendations

### Immediate Term
1. ✅ **CPU-only mode** - Current working solution (slow but functional)
2. **Remote inference** - Use Gemini API or other cloud LLMs
3. **Different hardware** - Raspberry Pi 5, Jetson Orin Nano, or x86 with proper GPU

### Medium Term (if UNO Q usage is required)
1. **Contact Arduino Support** - Ask about official Hexagon NPU access
2. **Wait for Mesa Turnip fixes** - Track Mesa development for Adreno improvements
3. **Build kernel with FastRPC device tree** - If cross-compilation becomes available

### Long Term
1. **Qualcomm vendor support** - Request FastRPC/Hexagon SDK for UNO Q
2. **Arduino firmware update** - Request device tree with FastRPC nodes
3. **Upstream Mesa contributions** - Help fix Turnip driver for llama.cpp

## Conclusion

The Arduino UNO Q has capable hardware (Adreno 702 GPU, Hexagon V55 NPU) but the software stack has fundamental blockers:

- **GPU path**: Mesa Turnip driver bugs → C++ abort, cannot be caught
- **OpenCL path**: Mesa doesn't support Adreno (needs proprietary driver)
- **NPU path**: Missing device tree configuration (needs kernel rebuild)

All fixes require either kernel/driver modifications or proprietary SDKs, both blocked by build constraints (no Docker, OOM on native builds).

**Hardware acceleration is not viable on Arduino UNO Q at this time. CPU-only mode is the only working solution.**

---

**Investigation Date**: 2025-10-25
**llama.cpp Version**: 5d195f17bc (Oct 25, 2025)
**Mesa Version**: 25.1.0-1qcom1
**Kernel Version**: 6.16.0-geffa8626771a
**Status**: Complete - All paths exhausted
