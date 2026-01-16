# Hello World Runbook

Step-by-step guide to running your first agent.

## What This Demonstrates

- Single agent startup and configuration
- Local model loading (ministral-3b)
- Basic task execution

## Prerequisites

1. Build the binary:
   ```bash
   cd /path/to/arkavo-edge
   cargo build
   ```

2. Verify the binary exists:
   ```bash
   ls target/debug/arkavo
   ```

## Step-by-Step Execution

### Step 1: Navigate to Example

```bash
cd examples/01-hello-world
```

### Step 2: Run the Agent

```bash
./run.sh
```

**What to watch for:**
- "Loading model ministral-3b..." (first run downloads ~1.5GB)
- "Agent hello-agent started on port XXXX"
- Agent response to the greeting task

### Step 3: Observe Output

You should see output like:
```
Starting hello-agent...
Loading model: ministral-3b
Agent started on port 52341
Processing task: Introduce yourself

Hello! I'm hello-agent, a friendly AI assistant. I'm here to help
answer your questions and have a conversation. What would you like
to talk about?
```

### Step 4: Stop the Agent

Press `Ctrl+C` to stop the agent.

## Troubleshooting

### Model Download Fails

If the model doesn't download:
```bash
# Check internet connection
curl -I https://huggingface.co

# Try with debug logging
RUST_LOG=debug ./run.sh
```

### Port Already in Use

The agent uses dynamic port assignment. If you see port errors:
```bash
# Kill any orphan processes
pkill -f "arkavo agent"
```

### Binary Not Found

```bash
# Rebuild
cd /path/to/arkavo-edge
cargo build
```

## Architecture Notes

This example uses the simplest possible configuration:
- Single agent (no mesh)
- Local model (no API keys needed)
- mDNS enabled (for future discovery)
- Dynamic port (OS assigns available port)

## Verification

Success criteria:
- Agent starts without errors
- Agent responds to the greeting task
- Agent stops cleanly with Ctrl+C
