# Hello World Runbook

Step-by-step guide to your first Arkavo response.

## What This Demonstrates

- Simple chat interaction with a local model
- Basic response without tools
- Quick validation that your setup works

## Prerequisites

An `arkavo` binary. Check for an installed one:

```bash
arkavo --version
```

If none is installed, install it as described in the [top-level README](../../README.md#quick-start), or build it from a source checkout:

```bash
cd /path/to/arkavo-edge
cargo build
ls target/debug/arkavo
```

`run.sh` uses `$BINARY` if set, then the source build, then `arkavo` on `PATH`.

## Step-by-Step Execution

### Navigate to the Example

```bash
cd examples/01-hello-world
```

### Run the Script

```bash
./run.sh
```

The script runs one `arkavo chat --prompt` command and exits when the response is complete.

**What to watch for:**
- On the first run with no local models downloaded, `arkavo` lists two models and asks before downloading them: Gemma 4 E2B (3.0 GB) for routing, and Gemma 4 12B (7.4 GB) for inference on a desktop or workstation, 10.4 GB in total. On a device with less than 16 GB of RAM the second model is Gemma 4 E4B (5.0 GB), 8.0 GB in total.
- After the download `arkavo` exits without answering. Run `./run.sh` again.
- The first-run prompt needs an interactive terminal. Without one, download the models first with `arkavo model download`.
- A friendly greeting response.

### Observe the Output

You should see output like:
```
Hello World Agent
=================

Starting hello-agent...

Hello! I'm here to help. What can I do for you?
```

The exact wording may vary, but you should get a friendly greeting response. The script then returns to the shell; there is no process to stop.

### Optional: Run the Kit as an Agent

`run.sh` does not start an agent. To run the role defined in `hello-agent.swarmkit.yaml` as a long-lived agent:

```bash
arkavo model download ministral-3b   # once, if the model is not cached
arkavo agent -c hello-agent.swarmkit.yaml -v
```

Press `Ctrl+C` to stop it.

## Troubleshooting

### Model Download Fails

If the model doesn't download:
```bash
# Check internet connection
curl -I https://huggingface.co

# Try with debug logging
RUST_LOG=debug ./run.sh
```

### Unknown Model Warning

`--model` takes a catalog name such as `ministral-3b`, `gemma-4-12b`, `qwen3.5-9b`, or `glm-4.7-flash`, or a path to a `.gguf` file. A name the CLI does not know produces a warning and the default model is used.

### Binary Not Found

`run.sh` prints where it looked. Install `arkavo`, build it from source, or point the script at a binary:

```bash
BINARY=/path/to/arkavo ./run.sh
```

## Architecture Notes

`run.sh` is a one-shot chat:
- No agent process, no mesh, no listening port
- Local model (no API keys needed)
- The kit in this directory supplies `runtime` settings to `arkavo chat`; its role, model, and instructions are used only when the kit is run with `arkavo agent -c`

## Verification

Success criteria:
- The script runs without errors
- A greeting response is printed
- The script exits on its own
