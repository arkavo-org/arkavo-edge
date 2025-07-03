# Local LLM Support

Arkavo Edge supports running language models locally using the Candle ML framework. This provides privacy-first, offline-capable AI inference directly on your machine.

## Prerequisites

- Rust with the `local` feature enabled
- macOS (Apple Silicon) or Linux (x64/aarch64)
- At least 4GB of available RAM for small models

## Building with Local Support

To enable local LLM support, build with the feature flag:

```bash
cargo build --features arkavo-llm/local
```

## Model Management

Arkavo provides a `model` command to manage local models:

### List Available Models

```bash
arkavo model list
```

Shows all available and downloaded models, indicating which one is active.

### Add a Local Model

```bash
arkavo model add /path/to/model.gguf --name my-model
```

Adds a GGUF format model from your local filesystem.

### Switch Active Model

```bash
arkavo model switch my-model
```

Sets the specified model as the active one for inference.

### Download Model (Coming Soon)

```bash
arkavo model download gemma3n-e2b
```

Downloads a model from the configured registry.

## Architecture

The local LLM support is implemented through:

1. **LocalProvider**: Implements the Provider trait for local inference
2. **LocalProviderFactory**: Integrates with the existing provider system
3. **Model Registry**: Stores model metadata in the arkavo-memory system
4. **Device Selection**: Automatically uses Metal on macOS M-series, CPU elsewhere

## Model Storage

Models are stored in:
- Default location: `$HOME/.arkavo/models`
- Override with: `ARKAVO_MODELS_PATH` environment variable

Model metadata is stored in the arkavo-memory database under the `models.local` key.

## Supported Formats

Currently supported:
- GGUF (Quantized models)

Planned:
- Safetensors

## Hardware Acceleration

- **macOS (Apple Silicon)**: Uses Metal Performance Shaders via Candle
- **Linux/Other**: CPU-based inference with optimized kernels

## Phase 1 Implementation Status

✅ Completed:
- Local feature flag and Candle dependencies
- LocalProvider module structure
- Provider trait implementation
- LocalProviderFactory integration
- Model command structure
- Basic model management (list, add, switch)

🚧 In Progress:
- Actual model inference implementation
- Model downloading from registry
- Memory limit protections

## Next Steps

Phase 2 will add:
- Model download manager
- Metal performance optimizations
- SHA256 verification
- Progress reporting

Phase 3 will add:
- Budget-aware model selection
- Resource protection (memory limits)
- Provider switching logic

Phase 4 will add:
- Gemma compliance
- Full documentation
- Production polish