# Arduino UNO Q (QRB2210) Deployment Guide

This guide covers deploying Arkavo Edge on Arduino UNO Q (Dragonwing QRB2210) with Qualcomm SNPE for hardware-accelerated ML inference.

## Important: Dynamic Loading

**Pre-built binaries include SNPE support via dynamic loading.**

Arkavo Edge uses `dlopen` to load the SNPE SDK at runtime. No build required - just install the SDK and the binary will auto-detect it. See `docs/uno-q-quickstart.md` for quick start instructions.

## Hardware Specifications

**Arduino UNO Q (Dragonwing QRB2210)**
- **SoC**: Qualcomm QRB2210 (Cortex-A53 quad-core @ 2.0 GHz)
- **GPU**: Adreno 702
- **DSP**: Hexagon 685 DSP with HTP (Hexagon Tensor Processor)
- **RAM**: 2GB LPDDR4
- **Storage**: 8GB eMMC (expandable via microSD)
- **OS**: Debian Linux 64-bit (aarch64)

**Performance Expectations**
- **Inference latency**: ≤ 50 ms for 224×224 input on GPU (models ≤ 10M params)
- **Throughput**: ≥ 20 FPS for real-time vision models
- **Memory usage**: ≤ 1 GB for continuous inference
- **Thermal**: < 70°C sustained operation

## Prerequisites

### SNPE SDK Installation

Download and install Qualcomm SNPE SDK from [Qualcomm Developer Network](https://developer.qualcomm.com/software/qualcomm-neural-processing-sdk):

```bash
# On development machine (x86_64 Linux)
wget https://developer.qualcomm.com/downloads/qualcomm-neural-processing-sdk-linux-v2.x.tar.gz
tar -xzf qualcomm-neural-processing-sdk-linux-v2.x.tar.gz
export SNPE_ROOT=$PWD/snpe-2.x

# Verify installation
source $SNPE_ROOT/bin/envsetup.sh
snpe-dlc-info --version
```

**License Compliance**: SNPE SDK is proprietary software. Review and accept Qualcomm's license agreement. Do not redistribute SNPE binaries.

### System Requirements

**Development Machine**:
- Linux x86_64 (for model conversion)
- Python 3.8+
- SNPE SDK installed
- Rust toolchain with aarch64-linux-gnu target

**UNO Q Device**:
- Debian Linux 64-bit
- SSH access enabled
- Minimum 2GB free storage
- Network connectivity

## Model Conversion Pipeline

Convert models to SNPE DLC (Deep Learning Container) format:

### Step 1: Export Model to ONNX

**PyTorch Example**:
```python
import torch
import torch.onnx

model = torch.load('model.pt')
model.eval()

dummy_input = torch.randn(1, 3, 224, 224)

torch.onnx.export(
    model,
    dummy_input,
    "model.onnx",
    export_params=True,
    opset_version=11,
    do_constant_folding=True,
    input_names=['input'],
    output_names=['output'],
    dynamic_axes={
        'input': {0: 'batch_size'},
        'output': {0: 'batch_size'}
    }
)
```

**TensorFlow/Keras Example**:
```python
import tensorflow as tf
import tf2onnx

model = tf.keras.models.load_model('model.h5')

spec = (tf.TensorSpec((None, 224, 224, 3), tf.float32, name="input"),)
output_path = "model.onnx"

model_proto, _ = tf2onnx.convert.from_keras(model, input_signature=spec, output_path=output_path)
```

### Step 2: Convert ONNX to DLC

Use the provided conversion script:

```bash
# Basic conversion with INT8 quantization for GPU
./scripts/convert-to-snpe.sh models/gemma-3-270m.onnx

# FP16 quantization for DSP
./scripts/convert-to-snpe.sh --quantize fp16 --target dsp models/model.onnx

# No quantization (debugging)
./scripts/convert-to-snpe.sh --quantize none models/model.onnx
```

**Manual Conversion**:
```bash
export SNPE_ROOT=/path/to/snpe-sdk
source $SNPE_ROOT/bin/envsetup.sh

# Convert ONNX to DLC
snpe-onnx-to-dlc \
    --input_network model.onnx \
    --output_path model.dlc

# Quantize to INT8
snpe-dlc-quantize \
    --input_dlc model.dlc \
    --output_dlc model_int8.dlc \
    --input_list calibration_data.txt \
    --optimizations cle

# Validate DLC
snpe-dlc-info --input_dlc model_int8.dlc
```

### Step 3: Validate Model

```bash
# Check model info
snpe-dlc-info --input_dlc model.dlc

# Run benchmark on development machine (CPU only)
snpe-net-run --container model.dlc --input_list inputs.txt

# Profile model layers
snpe-dlc-viewer --input_dlc model.dlc --output_dir profile/
```

## Cross-Compilation

Build Arkavo Edge for UNO Q (aarch64-linux):

```bash
# Install cross-compilation toolchain
rustup target add aarch64-unknown-linux-gnu

# Install linker
sudo apt install gcc-aarch64-linux-gnu

# Configure Cargo
cat >> ~/.cargo/config.toml <<EOF
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
EOF

# Build
cargo build --release --target aarch64-unknown-linux-gnu -p arkavo --features snpe

# Binary location
ls -lh target/aarch64-unknown-linux-gnu/release/arkavo
```

## Deployment

Use the automated provisioning script:

```bash
# Deploy with model
./scripts/provision-uno-q.sh \
    --user debian \
    --model models/gemma-3-270m.dlc \
    --snpe $SNPE_ROOT \
    192.168.1.100

# Or manual deployment
UNO_Q_IP=192.168.1.100
scp target/aarch64-unknown-linux-gnu/release/arkavo debian@$UNO_Q_IP:/opt/arkavo/bin/
scp models/gemma-3-270m.dlc debian@$UNO_Q_IP:/opt/arkavo/models/
```

### Manual Installation Steps

If the provisioning script fails, follow these manual steps:

```bash
# SSH into UNO Q
ssh debian@192.168.1.100

# Create directories
sudo mkdir -p /opt/{arkavo/{bin,models,logs},snpe/lib}
sudo chown -R debian:debian /opt/arkavo

# Copy SNPE runtime (from development machine)
# On dev machine:
scp -r $SNPE_ROOT/lib/aarch64-linux debian@192.168.1.100:/opt/snpe/lib/

# On UNO Q:
# Configure environment
cat >> ~/.bashrc <<'EOF'
export SNPE_ROOT=/opt/snpe
export LD_LIBRARY_PATH=$SNPE_ROOT/lib/aarch64-linux:${LD_LIBRARY_PATH:-}
export ARKAVO_SNPE_RUNTIME=GPU_FP16
export ARKAVO_MODEL_PATH=/opt/arkavo/models
export ARKAVO_DEBUG=1
EOF

source ~/.bashrc

# Verify SNPE installation
ls -lh $SNPE_ROOT/lib/aarch64-linux/libSNPE.so

# Test Arkavo binary
/opt/arkavo/bin/arkavo --version
```

## Configuration

### Environment Variables

Create `/opt/arkavo/arkavo-env`:

```bash
# SNPE SDK
export SNPE_ROOT=/opt/snpe
export LD_LIBRARY_PATH=$SNPE_ROOT/lib/aarch64-linux:${LD_LIBRARY_PATH:-}

# Accelerator selection (priority: GPU > DSP > CPU)
export ARKAVO_SNPE_RUNTIME=GPU_FP16  # Options: GPU_FP16, GPU, DSP, CPU

# Model configuration
export ARKAVO_MODEL_PATH=/opt/arkavo/models/gemma-3-270m.dlc

# Debugging
export ARKAVO_DEBUG=1
export RUST_LOG=info

# Performance tuning
export ARKAVO_MAX_THREADS=4
export ARKAVO_BATCH_SIZE=1
```

### Systemd Service

Create `/etc/systemd/system/arkavo.service`:

```ini
[Unit]
Description=Arkavo Edge Agent with SNPE
After=network.target

[Service]
Type=simple
User=debian
WorkingDirectory=/opt/arkavo
EnvironmentFile=/opt/arkavo/arkavo-env
ExecStart=/opt/arkavo/bin/arkavo
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable arkavo
sudo systemctl start arkavo

# Check status
sudo systemctl status arkavo

# View logs
journalctl -u arkavo -f
```

## Testing and Validation

### Basic Inference Test

```bash
# Interactive chat
arkavo chat --model snpe --prompt "What is 2+2?"

# Single inference
arkavo chat --model snpe --prompt "Hello" --max-tokens 50
```

### Performance Benchmark

```bash
# Latency test
time arkavo chat --model snpe --prompt "test" --max-tokens 10

# Throughput test
arkavo bench --model snpe --iterations 100 --input-size 224

# Expected: < 50ms latency, > 20 FPS throughput
```

### Thermal Monitoring

```bash
# Install monitoring tools
sudo apt install lm-sensors sysstat

# Monitor CPU temperature
watch -n 1 cat /sys/class/thermal/thermal_zone0/temp

# Monitor GPU usage
watch -n 1 cat /sys/class/kgsl/kgsl-3d0/gpubusy

# Run sustained inference test (1 hour)
timeout 3600 arkavo chat --model snpe --prompt "long context test" --max-tokens 1000
```

### Memory Profiling

```bash
# Monitor memory during inference
pidstat -r -p $(pgrep arkavo) 1

# Check for memory leaks
valgrind --leak-check=full /opt/arkavo/bin/arkavo chat --model snpe --prompt "test"
```

## Performance Tuning

### GPU Optimization

```bash
# Set GPU governor to performance
echo performance | sudo tee /sys/class/kgsl/kgsl-3d0/devfreq/governor

# Check GPU frequency
cat /sys/class/kgsl/kgsl-3d0/devfreq/cur_freq
cat /sys/class/kgsl/kgsl-3d0/devfreq/max_freq
```

### CPU Governor

```bash
# Set CPU to performance mode
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Verify
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor
```

### Storage Optimization

```bash
# Use microSD for models (if available)
sudo mkdir -p /mnt/sd/arkavo/models
sudo chown debian:debian /mnt/sd/arkavo
mv /opt/arkavo/models/* /mnt/sd/arkavo/models/
ln -s /mnt/sd/arkavo/models /opt/arkavo/models
```

## Troubleshooting

### SNPE Library Not Found

```bash
# Error: libSNPE.so: cannot open shared object file

# Solution: Verify LD_LIBRARY_PATH
echo $LD_LIBRARY_PATH
ls -lh $SNPE_ROOT/lib/aarch64-linux/libSNPE.so

# Add to environment
export LD_LIBRARY_PATH=/opt/snpe/lib/aarch64-linux:$LD_LIBRARY_PATH
```

### GPU Not Available

```bash
# Error: Accelerator GPU_FP16 is not available

# Check GPU device
ls -l /dev/kgsl-3d0
cat /sys/class/kgsl/kgsl-3d0/gpubusy

# Fallback to DSP or CPU
export ARKAVO_SNPE_RUNTIME=DSP
# or
export ARKAVO_SNPE_RUNTIME=CPU
```

### Model Load Failed

```bash
# Error: Failed to load DLC model

# Validate DLC file
snpe-dlc-info --input_dlc model.dlc

# Check file permissions
ls -lh /opt/arkavo/models/model.dlc
chmod 644 /opt/arkavo/models/model.dlc

# Verify model architecture matches SNPE version
snpe-dlc-info --input_dlc model.dlc | grep "SNPE Version"
```

### Out of Memory

```bash
# Error: Memory allocation failed

# Check available memory
free -h

# Reduce batch size
export ARKAVO_BATCH_SIZE=1

# Use INT8 quantization instead of FP16
# Reconvert model with --quantize int8
```

## Acceptance Criteria Validation

Run the full validation suite:

```bash
# 1. Performance: Latency ≤ 50ms
time arkavo chat --model snpe --prompt "test" --max-tokens 10

# 2. Throughput: ≥ 20 FPS
arkavo bench --model snpe --fps-target 20

# 3. Memory: ≤ 1GB
pidstat -r -p $(pgrep arkavo) 1 10

# 4. CPU utilization: ≤ 40%
pidstat -u -p $(pgrep arkavo) 1 10

# 5. Reliability: 1 hour continuous test
timeout 3600 arkavo chat --model snpe --prompt "sustained test" --loop

# 6. Thermal: < 70°C
watch -n 1 cat /sys/class/thermal/thermal_zone0/temp
```

## Next Steps

- **Multi-model deployment**: Deploy vision + language models
- **Edge mesh networking**: Connect multiple UNO Q devices
- **Remote monitoring**: Set up Prometheus metrics
- **Over-the-air updates**: Implement model versioning and rollback

## References

- [Qualcomm SNPE Documentation](https://developer.qualcomm.com/software/qualcomm-neural-processing-sdk/learning-resources)
- [Arduino UNO Q Specifications](https://www.arduino.cc/en/hardware/uno-q)
- [Arkavo Edge Repository](https://github.com/arkavo-org/arkavo-edge)
- [ONNX Model Zoo](https://github.com/onnx/models)
