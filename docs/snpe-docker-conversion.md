# SNPE Model Conversion with Docker

Quick guide for converting ONNX models to SNPE DLC format using Docker on any platform (macOS, Linux, Windows).

## Why Docker?

- **Cross-platform**: Works on macOS, Linux, Windows
- **No SDK installation**: SNPE SDK stays local, mounted into container
- **Reproducible**: Same environment every time
- **Fast**: Conversion takes 5-10 minutes for 600M parameter models

## Prerequisites

**Required:**
- Docker Desktop installed and running
- SNPE SDK downloaded at `vendor/qairt/2.39.0.250926/` (gitignored)
- ONNX model with static shapes (no dynamic dimensions)

**Get SNPE SDK:**
1. Visit https://developer.qualcomm.com/software/qualcomm-neural-processing-sdk
2. Create account and download SNPE SDK 2.39.0+
3. Extract to `vendor/qairt/2.39.0.250926/` in your project

## Quick Start

```bash
# Clone repository
git clone https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge

# Prepare ONNX model (must have static shapes)
# Example: Qwen3-0.6B with seq_length=64
python3 scripts/fix-onnx-static-shapes.py \
  input.onnx \
  output_static.onnx \
  --seq-length 64

# Convert ONNX to DLC
./scripts/snpe-docker-convert.sh \
  output_static.onnx \
  model.dlc

# Output: model.dlc ready for deployment!
```

## Detailed Usage

### Convert Qwen3-0.6B Model

```bash
# 1. Download and fix ONNX model
hf download onnx-community/Qwen3-0.6B-ONNX \
  --local-dir ~/.cache/arkavo/models/qwen3-onnx \
  --include "onnx/model_int8.onnx" \
  --include "onnx/model.onnx_data"

python3 scripts/fix-onnx-static-shapes.py \
  ~/.cache/arkavo/models/qwen3-onnx/onnx/model_int8.onnx \
  ~/.cache/arkavo/models/onnx/qwen3-0.6b_static_seq64.onnx \
  --seq-length 64

# 2. Convert to DLC (takes 5-10 minutes)
./scripts/snpe-docker-convert.sh \
  ~/.cache/arkavo/models/onnx/qwen3-0.6b_static_seq64.onnx \
  /tmp/qwen3-0.6b_seq64.dlc

# 3. Verify output
ls -lh /tmp/qwen3-0.6b_seq64.dlc
# Expected: ~300-400MB DLC file
```

### Convert Custom Model

```bash
# Your ONNX model must have static shapes!
./scripts/snpe-docker-convert.sh \
  path/to/your/model.onnx \
  path/to/output.dlc
```

## Script Details

**Location:** `scripts/snpe-docker-convert.sh`

**What it does:**
1. Validates SNPE SDK exists at `vendor/qairt/2.39.0.250926/`
2. Checks Docker is running
3. Builds minimal Ubuntu 22.04 Docker image with Python
4. Mounts SNPE SDK (read-only) into container at `/snpe`
5. Mounts input ONNX and output directory
6. Runs `snpe-onnx-to-dlc` inside container
7. Displays DLC info and file size
8. Outputs DLC to your specified location

**Docker image:**
- Base: ubuntu:22.04
- Size: ~200MB
- Cached after first build
- Rebuilt only if Dockerfile changes

## Deploy to Arduino UNO Q

```bash
# Transfer DLC to device
adb push /tmp/qwen3-0.6b_seq64.dlc /data/local/tmp/

# Verify transfer
adb shell "ls -lh /data/local/tmp/qwen3-0.6b_seq64.dlc"

# Test inference (requires SNPE runtime on device)
adb shell "cd /data/local/tmp && snpe-net-run \
  --container qwen3-0.6b_seq64.dlc \
  --use_gpu"
```

## Troubleshooting

### Docker not running

```bash
# Check Docker status
docker info

# Start Docker Desktop if not running
# macOS: Open Docker Desktop app
# Linux: sudo systemctl start docker
# Windows: Start Docker Desktop
```

### SNPE SDK not found

```bash
# Verify SDK location
ls -la vendor/qairt/2.39.0.250926/bin/x86_64-linux-clang/

# Should contain:
# - snpe-onnx-to-dlc (Python script)
# - snpe-dlc-info
# - Other SNPE tools
```

### ONNX model has dynamic shapes

```bash
# Check for dynamic dimensions
python3 -c "
import onnx
model = onnx.load('model.onnx')
for inp in model.graph.input:
    for dim in inp.type.tensor_type.shape.dim:
        if dim.HasField('dim_param'):
            print(f'Dynamic: {dim.dim_param}')
"

# Fix with static shape script
python3 scripts/fix-onnx-static-shapes.py \
  model.onnx \
  model_static.onnx \
  --seq-length 64
```

### Conversion fails

**Common issues:**

1. **Unsupported ONNX operators**
   - Error: `Unsupported operator: ...`
   - Solution: Simplify model architecture or use different model

2. **KV cache tensors**
   - Error: Too many inputs/outputs
   - Solution: Export model with `use_cache=False`

3. **Memory error**
   - Error: Out of memory
   - Solution: Increase Docker memory limit in Docker Desktop settings

4. **Python dependencies missing**
   - Error: `ModuleNotFoundError`
   - Solution: SNPE SDK includes all dependencies, no extra pip install needed

## Performance Expectations

**Conversion time (on typical laptop):**
- Small models (<100M params): 1-2 minutes
- Medium models (100-500M params): 3-5 minutes
- Large models (500M-1B params): 5-10 minutes
- Very large models (>1B params): 10-20 minutes

**Output size:**
- FP32 DLC: ~same size as ONNX (~4 bytes per parameter)
- INT8 quantized: ~1/4 size of FP32 (requires calibration data)

**Example: Qwen3-0.6B**
- Input ONNX: 589MB (INT8 weights)
- Output DLC: ~300-400MB (FP32 converted)
- Conversion time: ~8 minutes on M1 Mac

## Advanced: INT8 Quantization

For DSP acceleration, quantize DLC to INT8:

```bash
# 1. Generate calibration data (Python)
python3 - <<'EOF'
import numpy as np
# Create sample inputs (100-500 samples recommended)
input_ids = np.random.randint(0, 151936, size=(500, 64), dtype=np.int64)
np.save('input_samples.npy', input_ids)
EOF

# Create input list
echo "input_ids:=input_samples.npy" > input_list.txt

# 2. Quantize DLC (add to Docker script)
docker run --rm \
  -v "$PWD/vendor/qairt/2.39.0.250926:/snpe:ro" \
  -v "$PWD:/workspace" \
  snpe-converter \
  bash -c "
    export SNPE_ROOT=/snpe
    export PATH=/snpe/bin/x86_64-linux-clang:\$PATH

    /snpe/bin/x86_64-linux-clang/snpe-dlc-quantize \
      --input_dlc model.dlc \
      --output_dlc model_int8.dlc \
      --input_list input_list.txt \
      --use_enhanced_quantizer
  "
```

## Next Steps

- **Model deployment**: See [uno-q-quickstart.md](uno-q-quickstart.md)
- **SNPE runtime setup**: See [snpe-model-conversion.md](snpe-model-conversion.md)
- **Performance tuning**: Benchmark GPU vs DSP vs CPU runtimes

## References

- SNPE SDK Documentation: https://developer.qualcomm.com/sites/default/files/docs/snpe/
- Supported ONNX Operators: https://developer.qualcomm.com/sites/default/files/docs/snpe/model_conv_onnx.html
- Arduino UNO Q Specs: https://docs.arduino.cc/hardware/uno-q/
