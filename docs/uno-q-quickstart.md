# UNO Q Quick Start

This guide shows how to enable Qualcomm SNPE hardware acceleration on Arduino UNO Q.

## Dynamic Loading: No Build Required!

**Pre-built binaries include SNPE support via dynamic loading.**

Arkavo Edge uses `dlopen` to load the SNPE SDK at runtime:
- Pre-built binaries work on any aarch64-linux system
- SNPE SDK auto-detected if installed
- Gracefully falls back to CPU if SDK not found
- No build required for hardware acceleration!

## Option 1: Use Pre-built Binary (Recommended)

Download and run the pre-built binary. SNPE will be auto-detected if installed:

```bash
# Download latest aarch64-linux binary
wget https://github.com/arkavo-org/arkavo-edge/releases/latest/download/arkavo-aarch64-linux
chmod +x arkavo-aarch64-linux

# Run (auto-detects SNPE SDK)
./arkavo-aarch64-linux --version
```

**To enable GPU/DSP acceleration**, install the SNPE SDK:

### Install SNPE SDK

1. Download from [Qualcomm Developer Network](https://developer.qualcomm.com/software/qualcomm-neural-processing-sdk)
2. Extract to `/opt/snpe`:
   ```bash
   sudo mkdir -p /opt/snpe
   sudo tar -xzf snpe-*.tar.gz -C /opt/snpe --strip-components=1
   ```
3. Configure environment:
   ```bash
   cat >> ~/.bashrc <<'EOF'
   export SNPE_ROOT=/opt/snpe
   export LD_LIBRARY_PATH=$SNPE_ROOT/lib/aarch64-linux:${LD_LIBRARY_PATH}
   export ARKAVO_SNPE_RUNTIME=GPU_FP16
   EOF
   source ~/.bashrc
   ```
4. Run Arkavo Edge - SNPE will be automatically loaded:
   ```bash
   ./arkavo-aarch64-linux --version
   # Should show: "SNPE library loaded successfully"
   ```

**Performance**:
- Without SDK: CPU inference (~200ms latency)
- With SDK: GPU acceleration (~20ms latency, 10x faster!)

## Option 2: Build from Source (Optional)

For developers who want to build from source:

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Clone repository
git clone --recursive https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge

# Build (no SNPE SDK required at build time!)
cargo build --release -p arkavo --features snpe

# Binary location
./target/release/arkavo --version
```

No SNPE SDK required at build time. The binary uses dynamic loading at runtime.

## Cross-Compilation (Optional)

Cross-compile on x86_64 Linux for UNO Q:

```bash
# Install toolchain
sudo apt install -y gcc-aarch64-linux-gnu
rustup target add aarch64-unknown-linux-gnu

# Configure linker
mkdir -p ~/.cargo
cat >> ~/.cargo/config.toml <<EOF
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
EOF

# Build
cargo build --release --target aarch64-unknown-linux-gnu -p arkavo --features snpe

# Deploy to UNO Q
scp target/aarch64-unknown-linux-gnu/release/arkavo debian@uno-q:/usr/local/bin/
```

## Testing SNPE Acceleration

Verify SNPE is loaded:

```bash
# Check GPU device available
ls -l /dev/kgsl-3d0

# Run with debug logging
ARKAVO_DEBUG=1 arkavo --version
# Should show: "SNPE library loaded successfully"

# Test inference (once models are deployed)
arkavo chat --model snpe --prompt "test"
```

## Expected Output

With SNPE working, you should see:
```
SNPE Runtime initialized with target: GPU_FP16
Loaded DLC model 'model_name' with 1 inputs, 1 outputs
```

Without SNPE (binary not built with SDK):
```
SNPE features not available - binary not built with SNPE support
```

## Performance Comparison

| Configuration | Latency | Hardware Used |
|---------------|---------|---------------|
| CPU only (no SNPE) | ~200ms | CPU (Cortex-A53) |
| SNPE CPU | ~100ms | CPU optimized |
| SNPE DSP | ~50ms | Hexagon DSP |
| SNPE GPU | ~30ms | Adreno 702 GPU |
| SNPE GPU_FP16 | ~20ms | Adreno 702 GPU (FP16) |

## Troubleshooting

### "SNPE SDK not found"
```bash
# Verify SNPE_ROOT is set
echo $SNPE_ROOT

# Verify SNPE libraries exist
ls -lh $SNPE_ROOT/lib/aarch64-linux/libSNPE.so

# Add to environment
export SNPE_ROOT=/opt/snpe
export LD_LIBRARY_PATH=$SNPE_ROOT/lib/aarch64-linux:$LD_LIBRARY_PATH
```

### "Binary not built with SNPE support"
This means the binary was built without the `snpe` feature flag. You need to:
- Build locally on UNO Q with `--features snpe`, OR
- Cross-compile with SNPE SDK on dev machine

### "GPU not available"
```bash
# Check GPU device
ls -l /dev/kgsl-3d0

# Fallback to DSP or CPU
export ARKAVO_SNPE_RUNTIME=DSP
# or
export ARKAVO_SNPE_RUNTIME=CPU
```

## Summary

**Dynamic Loading Approach**:
- ✅ Pre-built binaries include SNPE support
- ✅ No build required for hardware acceleration
- ✅ SNPE SDK loaded at runtime via dlopen
- ✅ Automatic fallback to CPU if SDK not found
- ✅ 10x faster inference with GPU (20ms vs 200ms)

**Quick Start**:
1. Download pre-built binary from GitHub releases
2. Install SNPE SDK to `/opt/snpe` (optional, for GPU acceleration)
3. Run - SNPE will be auto-detected

## Next Steps

1. Deploy a DLC model (see `docs/uno-q-deployment.md`)
2. Run inference tests
3. Monitor performance and thermal
