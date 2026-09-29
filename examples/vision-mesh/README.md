# Vision Mesh

Image analysis mesh using Qwen3.5-27B with vision support.

## Prerequisites

Download the model and vision projector:

```bash
hf download unsloth/Qwen3.5-27B-GGUF Qwen3.5-27B-UD-Q6_K_XL.gguf
hf download unsloth/Qwen3.5-27B-GGUF mmproj-Qwen2.5-VL-7B-f16.gguf
```

`launch.sh` uses `$BINARY` if set, then the source build, then `arkavo` on `PATH`. Build only when running from source:

```bash
cargo build
```

## Usage

```bash
# Start the mesh
./launch.sh

# Check status
./launch.sh status

# Stop
./stop.sh
```

## Agents

| Agent | Port | Role |
|-------|------|------|
| orchestrator | 8418 | Routes vision tasks |
| vision-analyst | 8420 | Analyzes images |

## Testing Vision

`arkavo chat` has no option for attaching an image, so this example cannot be exercised from the command line with a local file. The `--image` flag shown in earlier versions of this README does not exist.

The router automatically discovers the mmproj file alongside the Qwen3.5-27B model
and enables vision support. No manual configuration needed.
