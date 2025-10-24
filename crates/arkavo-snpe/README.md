# arkavo-snpe

Qualcomm SNPE (Snapdragon Neural Processing Engine) backend for Arkavo Edge on Arduino UNO Q (QRB2210) platform.

## Important: Licensing and Distribution

**The SNPE feature is NOT included in pre-built binaries.**

The Qualcomm SNPE SDK is proprietary software that cannot be redistributed. Pre-built binaries distributed via:
- Homebrew
- macOS .pkg installers
- GitHub releases

...will NOT include SNPE support and will use CPU inference only.

## Building with SNPE

To enable hardware acceleration on UNO Q, you must:

1. **Obtain SNPE SDK**: Download from [Qualcomm Developer Network](https://developer.qualcomm.com/software/qualcomm-neural-processing-sdk)
2. **Accept License**: Review and accept Qualcomm's license agreement
3. **Build from Source**: Build Arkavo Edge with the `snpe` feature flag

```bash
export SNPE_ROOT=/path/to/snpe-sdk
cargo build --release -p arkavo --features snpe
```

For automated builds on UNO Q, use the provided build script:

```bash
bash scripts/build-uno-q.sh
```

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
