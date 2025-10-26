# arkavo-snpe

Qualcomm SNPE (Snapdragon Neural Processing Engine) backend for Arkavo Edge on Arduino UNO Q (QRB2210) platform.

## Dynamic Loading Architecture

**Pre-built binaries include SNPE support via dynamic loading.**

This crate uses `dlopen`/`dlsym` to load the SNPE SDK at runtime:

- **No build-time dependency** on SNPE SDK
- **Portable binaries** work on any aarch64-linux system
- **Automatic fallback** to CPU inference if SDK not installed
- **GPU/DSP acceleration** when SDK is available

### How It Works

1. **Build time**: No SNPE linking required, binary is portable
2. **Runtime**: Searches for `libSNPE.so` in:
   - `/opt/snpe/lib/aarch64-linux`
   - `$SNPE_ROOT/lib/aarch64-linux`
   - Paths in `$LD_LIBRARY_PATH`
3. **If found**: GPU/DSP hardware acceleration enabled
4. **If not found**: Gracefully falls back to CPU inference

## For End Users

Install Arkavo Edge via Homebrew, .pkg, or GitHub releases:

```bash
# Download pre-built binary
wget https://github.com/arkavo-org/arkavo-edge/releases/latest/download/arkavo-aarch64-linux
chmod +x arkavo-aarch64-linux

# Run (CPU-only if SNPE SDK not installed)
./arkavo-aarch64-linux --version
```

To enable SNPE acceleration, install the SDK:

1. Download SNPE SDK from [Qualcomm Developer Network](https://developer.qualcomm.com/software/qualcomm-neural-processing-sdk)
2. Extract to `/opt/snpe` or set `SNPE_ROOT`
3. Configure `LD_LIBRARY_PATH`:
   ```bash
   export LD_LIBRARY_PATH=/opt/snpe/lib/aarch64-linux:$LD_LIBRARY_PATH
   ```
4. Run Arkavo Edge - SNPE will be auto-detected

## For Developers

Build from source with `snpe` feature:

```bash
cargo build --release -p arkavo --features snpe
```

No SNPE SDK required at build time. The binary will dynamically load the SDK at runtime if available.

## Platform Support

This crate only builds on:
- **OS**: Linux
- **Architecture**: aarch64 (ARM 64-bit)

Attempting to build on other platforms will result in a no-op compilation.

## Hardware Acceleration

When built with SNPE support, Arkavo Edge can leverage:

- **Adreno 702 GPU**: FP16/FP32 inference (~20ms latency)
- **Hexagon DSP**: INT8 quantized inference (~50ms latency)
- **CPU**: Fallback for unsupported operations (~200ms latency)

Accelerator selection is automatic with priority: GPU_FP16 > GPU > DSP > CPU

## Documentation

See:
- `docs/uno-q-quickstart.md` - Quick start for end users and developers
- `docs/uno-q-deployment.md` - Complete deployment guide with model conversion

## License

This crate is part of Arkavo Edge and licensed under the same terms. Note that linking with the SNPE SDK requires acceptance of Qualcomm's separate license agreement.
