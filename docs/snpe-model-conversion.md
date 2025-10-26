# SNPE Model Conversion Guide

Complete guide for converting ONNX models to SNPE DLC format for hardware-accelerated inference on Qualcomm platforms (Arduino UNO Q, Raspberry Pi with Snapdragon, etc.).

## Overview

**SNPE (Snapdragon Neural Processing Engine)** enables hardware acceleration on Qualcomm chipsets:
- **GPU**: Adreno 702 (10-20ms latency, FP16 precision)
- **DSP**: Hexagon 685 HTP (20-50ms latency, INT8 quantization)
- **CPU**: Cortex-A53 fallback (100-200ms latency)

**Conversion Pipeline:**
```
PyTorch/TensorFlow → ONNX (static shapes) → DLC → Quantized DLC → Deploy
```

## Prerequisites

**On Development Machine:**
- Python 3.8+ with transformers, torch, onnx packages
- SNPE SDK 2.39.0+ (for conversion tools)
- ADB tools for deployment

**On Target Device (UNO Q):**
- SNPE runtime libraries (see [uno-q-quickstart.md](uno-q-quickstart.md))
- Sufficient storage (~1GB for model + cache)

## Step 1: Prepare ONNX Model with Static Shapes

SNPE **requires static shapes** - no dynamic dimensions allowed.

### Option A: Download Pre-converted ONNX

```bash
# Download Qwen3-0.6B ONNX model (has dynamic shapes)
hf download onnx-community/Qwen3-0.6B-ONNX \
  --local-dir ~/.cache/arkavo/models/qwen3-onnx \
  --include "onnx/model_int8.onnx" \
  --include "onnx/model.onnx_data"

# Convert dynamic shapes to static (batch=1, seq=64)
python3 scripts/fix-onnx-static-shapes.py \
  ~/.cache/arkavo/models/qwen3-onnx/onnx/model_int8.onnx \
  ~/.cache/arkavo/models/onnx/qwen3-0.6b_static_seq64.onnx \
  --seq-length 64

# Verify static shapes
python3 -c "
import onnx
model = onnx.load('~/.cache/arkavo/models/onnx/qwen3-0.6b_static_seq64.onnx')
for inp in model.graph.input:
    shape = [d.dim_value for d in inp.type.tensor_type.shape.dim]
    print(f'{inp.name}: {shape}')
"
```

### Option B: Export from PyTorch/Transformers

⚠️ **Known Issue**: Qwen3 and Gemma-3 use complex attention (vmap) that blocks ONNX export without KV cache.

For simpler models:
```bash
python3 scripts/export-to-onnx.py \
  --model <model-name> \
  --seq-length 64 \
  --output model_static.onnx
```

## Step 2: Convert ONNX to DLC

**Important**: SNPE conversion tools only run on **Linux** (not macOS/Windows).

### On Linux Development Machine

```bash
# Set SNPE environment
export SNPE_ROOT=/path/to/qairt/2.39.0.250926
export PATH=$SNPE_ROOT/bin/x86_64-linux-clang:$PATH

# Convert ONNX to DLC
snpe-onnx-to-dlc \
  -i qwen3-0.6b_static_seq64.onnx \
  -o qwen3-0.6b_seq64.dlc \
  --input_dim input_ids 1,64 \
  --input_dim attention_mask 1,64 \
  --input_dim position_ids 1,64

# Verify DLC
snpe-dlc-info -i qwen3-0.6b_seq64.dlc
```

### On UNO Q (Recommended for Edge Deployment)

```bash
# Transfer ONNX model to device
adb push qwen3-0.6b_static_seq64.onnx /data/local/tmp/

# SSH into device (or use adb shell)
ssh arduino@uno-q

# Set SNPE environment
export SNPE_ROOT=/opt/snpe
export PATH=$SNPE_ROOT/bin/aarch64-linux:$PATH

# Convert ONNX to DLC on device
cd /data/local/tmp
snpe-onnx-to-dlc \
  -i qwen3-0.6b_static_seq64.onnx \
  -o qwen3-0.6b_seq64.dlc \
  --input_dim input_ids 1,64

# Check conversion output
snpe-dlc-info -i qwen3-0.6b_seq64.dlc
```

**Expected Output:**
```
DLC info for qwen3-0.6b_seq64.dlc
  Inputs:
    input_ids: [1, 64] (INT64)
  Outputs:
    logits: [1, 64, 151936] (FLOAT32)
  Layers: 28 (transformer layers)
  Total parameters: 600M
```

## Step 3: Quantize DLC for DSP/GPU

Quantization reduces model size and improves inference speed on DSP/GPU.

```bash
# Generate sample inputs for calibration (INT8 quantization)
python3 - << 'EOF'
import numpy as np

# Create sample input_ids (random token IDs)
input_ids = np.random.randint(0, 151936, size=(100, 64), dtype=np.int64)
np.save('input_ids_samples.npy', input_ids)
EOF

# Create input list file
echo "input_ids:=input_ids_samples.npy" > input_list.txt

# Quantize DLC to INT8 for DSP
snpe-dlc-quantize \
  --input_dlc qwen3-0.6b_seq64.dlc \
  --output_dlc qwen3-0.6b_seq64_int8.dlc \
  --input_list input_list.txt \
  --use_enhanced_quantizer

# Verify quantized DLC
snpe-dlc-info -i qwen3-0.6b_seq64_int8.dlc
```

**Quantization Options:**
- `--use_enhanced_quantizer`: Better accuracy (recommended)
- `--optimizations CLE`: Cross-layer equalization for better quantization
- `--act_quantizer tf`: Use TensorFlow-style activation quantization

## Step 4: Deploy to UNO Q

```bash
# Create model directory on device
adb shell "mkdir -p /home/arduino/models"

# Transfer DLC models
adb push qwen3-0.6b_seq64.dlc /home/arduino/models/
adb push qwen3-0.6b_seq64_int8.dlc /home/arduino/models/

# Verify deployment
adb shell "ls -lh /home/arduino/models/*.dlc"
```

## Step 5: Test Inference

```bash
# SSH into device
ssh arduino@uno-q

# Load SNPE environment
source /home/arduino/arkavo-env.sh

# Test with FP32 DLC (GPU)
export ARKAVO_SNPE_RUNTIME=GPU_FP16
arkavo chat --model-path /home/arduino/models/qwen3-0.6b_seq64.dlc \
  --prompt "What is 2+2?"

# Test with INT8 DLC (DSP - faster)
export ARKAVO_SNPE_RUNTIME=DSP
arkavo chat --model-path /home/arduino/models/qwen3-0.6b_seq64_int8.dlc \
  --prompt "What is 2+2?"
```

## Troubleshooting

### "ONNX model has dynamic shapes"

```bash
# Check model shapes
python3 -c "
import onnx
model = onnx.load('model.onnx')
for inp in model.graph.input:
    for dim in inp.type.tensor_type.shape.dim:
        if dim.HasField('dim_param'):
            print(f'Dynamic dimension: {dim.dim_param}')
"

# Fix with static shape conversion script
python3 scripts/fix-onnx-static-shapes.py input.onnx output.onnx --seq-length 64
```

### "snpe-onnx-to-dlc: command not found"

```bash
# Verify SNPE SDK installation
echo $SNPE_ROOT
ls -l $SNPE_ROOT/bin/*/snpe-onnx-to-dlc

# Add to PATH
export PATH=$SNPE_ROOT/bin/x86_64-linux-clang:$PATH  # Linux dev machine
# or
export PATH=$SNPE_ROOT/bin/aarch64-linux:$PATH  # UNO Q
```

### "Unsupported ONNX operator"

Some ONNX operators are not supported by SNPE. Common issues:
- **KV cache tensors**: Set `past_sequence_length=0` to disable
- **Dynamic shapes**: Use static shapes only
- **vmap/custom ops**: Simplify model architecture

**Workaround**: Use pre-exported models from onnx-community or simplify architecture.

### "Quantization failed: insufficient samples"

```bash
# Generate more calibration samples (at least 100)
python3 - << 'EOF'
import numpy as np
input_ids = np.random.randint(0, 151936, size=(500, 64), dtype=np.int64)
np.save('input_ids_samples.npy', input_ids)
EOF

# Retry quantization
snpe-dlc-quantize --input_dlc model.dlc --output_dlc model_int8.dlc \
  --input_list input_list.txt --use_enhanced_quantizer
```

### "Model too large for device memory"

```bash
# Check available memory
adb shell "free -h"

# Reduce model size:
# 1. Use smaller sequence length (--seq-length 32)
# 2. Use INT8 quantization
# 3. Use smaller model (270M instead of 600M params)
```

## Performance Benchmarking

```bash
# Benchmark DLC inference
snpe-net-run \
  --container qwen3-0.6b_seq64_int8.dlc \
  --input_list input_list.txt \
  --use_dsp  # or --use_gpu

# Expected latency on UNO Q:
# - GPU_FP16: 20-30ms
# - DSP_INT8: 30-50ms
# - CPU: 100-200ms
```

## Model Size Guidelines

**For UNO Q (Cortex-A53, 2GB RAM):**
- **Recommended**: ≤1B parameters
- **Sequence length**: 32-128 tokens
- **Quantization**: INT8 for DSP, FP16 for GPU
- **Memory budget**: ≤1GB for model + runtime

**Tested Models:**
- Qwen3-0.6B: 600M params, 64 seq → 589MB ONNX, ~300MB DLC INT8
- Gemma-3-270M: 270M params, 64 seq → ~200MB ONNX (export blocked by vmap)

## Summary

**Successful Conversion Requirements:**
1. ✅ Static shapes (no dynamic dimensions)
2. ✅ Supported ONNX operators (no vmap, custom ops)
3. ✅ Reasonable model size (≤1B params for UNO Q)
4. ✅ SNPE SDK installed on Linux or target device
5. ✅ Calibration data for INT8 quantization

**Known Blockers:**
- ❌ Qwen3/Gemma-3 export without KV cache (vmap in attention)
- ❌ macOS/Windows SNPE tools (Linux only)
- ❌ Models >2GB on UNO Q (memory constraints)

**Next Steps:**
- Deploy quantized DLC to UNO Q
- Integrate with Arkavo Edge runtime
- Benchmark latency/throughput
- Measure power consumption

For runtime integration, see:
- [uno-q-quickstart.md](uno-q-quickstart.md) - Device setup
- [crates/arkavo-snpe/README.md](../crates/arkavo-snpe/README.md) - Runtime API
