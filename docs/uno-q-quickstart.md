# UNO Q Quick Start

This guide shows how to enable Qualcomm SNPE hardware acceleration on Arduino UNO Q.

## Important: SNPE SDK Licensing

The Qualcomm SNPE SDK is proprietary software that cannot be redistributed. Therefore:

**Pre-built binaries (Homebrew, .pkg, GitHub releases) do NOT include SNPE support.**

To use SNPE hardware acceleration on UNO Q, you must build from source with the SDK.

## Prerequisites

**On UNO Q**:
- Debian Linux 64-bit (aarch64)
- SNPE SDK installed from Qualcomm Developer Network
- SSH access enabled (for remote builds)
- Network connectivity

## For End Users: Pre-built Binary (No SNPE)

End users who install via Homebrew, .pkg, or GitHub releases will get a binary that works on UNO Q but uses CPU inference only:

```bash
# Download latest aarch64-linux binary
wget https://github.com/arkavo-org/arkavo-edge/releases/latest/download/arkavo-aarch64-linux
chmod +x arkavo-aarch64-linux

# Run (CPU inference only)
./arkavo-aarch64-linux --version
```

**Performance**: CPU-only inference (~200ms latency vs ~20ms with GPU acceleration)

## For Developers: Build with SNPE

To enable SNPE hardware acceleration, build from source on or for the UNO Q:

### Step 1: Obtain SNPE SDK

Download the Qualcomm Neural Processing SDK from:
https://developer.qualcomm.com/software/qualcomm-neural-processing-sdk

Extract to `/opt/snpe` on the UNO Q or your preferred location.

### Step 2: Install Build Dependencies

```bash
# On UNO Q
sudo apt update
sudo apt install -y build-essential cmake clang libclang-dev pkg-config curl git

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### Step 3: Set SNPE Environment

```bash
# Point to your SNPE SDK installation
export SNPE_ROOT=/opt/snpe  # or wherever you installed it

# Verify SNPE is accessible
ls -lh $SNPE_ROOT/lib/aarch64-linux/libSNPE.so
```

### Step 4: Clone and Build

```bash
# Clone repository
git clone --recursive https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge

# Build with SNPE support (use build script for interactive setup)
bash scripts/build-uno-q.sh

# Or build directly with cargo
cargo build --release -p arkavo --features snpe

# Binary location
./target/release/arkavo --version
```

The `build-uno-q.sh` script will automatically:
- Detect your SNPE SDK installation
- Verify required dependencies
- Build with appropriate feature flags
- Offer to install the binary system-wide

### Step 5: Install

```bash
# Copy to system location
sudo cp target/release/arkavo /usr/local/bin/
sudo chmod +x /usr/local/bin/arkavo

# Configure environment
cat >> ~/.bashrc <<'EOF'
export SNPE_ROOT=/opt/snpe
export LD_LIBRARY_PATH=$SNPE_ROOT/lib/aarch64-linux:${LD_LIBRARY_PATH}
export ARKAVO_SNPE_RUNTIME=GPU_FP16
EOF

source ~/.bashrc

# Verify installation
arkavo --version
```

## Advanced: Cross-Compile with SNPE

Advanced users can cross-compile on a development machine with SNPE SDK, then deploy to UNO Q:

### On Development Machine (x86_64 Linux)

```bash
# Install cross-compilation toolchain
sudo apt install -y gcc-aarch64-linux-gnu g++-aarch64-linux-gnu
rustup target add aarch64-unknown-linux-gnu

# Copy SNPE SDK to development machine
# Extract SNPE SDK to vendor/qairt/

# Set SNPE environment
export SNPE_ROOT=$PWD/vendor/qairt

# Configure Cargo for cross-compilation
cat >> ~/.cargo/config.toml <<EOF
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
EOF

# Build with SNPE
cargo build --release --target aarch64-unknown-linux-gnu -p arkavo --features snpe

# Binary location
ls -lh target/aarch64-unknown-linux-gnu/release/arkavo
```

### Deploy to UNO Q

```bash
# Copy binary
scp target/aarch64-unknown-linux-gnu/release/arkavo debian@uno-q:/tmp/

# Copy SNPE runtime libraries (only if UNO Q doesn't have them)
scp -r vendor/qairt/lib/aarch64-linux debian@uno-q:/tmp/snpe-lib/

# On UNO Q
ssh debian@uno-q

# Install binary
sudo mv /tmp/arkavo /usr/local/bin/
sudo chmod +x /usr/local/bin/arkavo

# Install SNPE libraries (if needed)
sudo mkdir -p /opt/snpe/lib
sudo mv /tmp/snpe-lib /opt/snpe/lib/aarch64-linux

# Configure environment
cat >> ~/.bashrc <<'EOF'
export SNPE_ROOT=/opt/snpe
export LD_LIBRARY_PATH=$SNPE_ROOT/lib/aarch64-linux:${LD_LIBRARY_PATH}
export ARKAVO_SNPE_RUNTIME=GPU_FP16
EOF

source ~/.bashrc

# Verify
arkavo --version
```

## Testing SNPE Acceleration

Once installed, verify SNPE is working:

```bash
# Check SNPE libraries are accessible
ldd /usr/local/bin/arkavo | grep SNPE

# Check GPU device
ls -l /dev/kgsl-3d0

# Set debug mode to see SNPE logs
export ARKAVO_DEBUG=1

# Run a simple inference
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

**For End Users (No Build Required)**:
- Download pre-built binary from GitHub releases
- Works on UNO Q with CPU inference
- No SNPE acceleration due to licensing restrictions

**For Developers (Hardware Acceleration)**:
- Build from source on UNO Q with SNPE SDK
- Enables GPU/DSP acceleration (10x faster)
- Requires Qualcomm SDK license agreement
- Use `scripts/build-uno-q.sh` for automated setup

**Build Time on UNO Q**:
- First build: ~20 minutes
- Incremental rebuilds: ~2-5 minutes
- One-time setup, updates as needed

## Next Steps

**End Users**:
1. Download arkavo-aarch64-linux from releases
2. Transfer to UNO Q and run

**Developers with SNPE SDK**:
1. Build on UNO Q using `scripts/build-uno-q.sh`
2. Deploy a DLC model (see `docs/uno-q-deployment.md`)
3. Run inference tests with hardware acceleration
4. Monitor performance and thermal characteristics
