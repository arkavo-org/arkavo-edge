# Hello World Runbook

Step-by-step guide to running your first agent.

## What This Demonstrates

- Simple chat interaction with a local model
- Basic agent response without tools
- Quick validation that your setup works

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
- First run may download the model (~1.5GB)
- A friendly greeting response from the agent

### Step 3: Observe Output

You should see output like:
```
Hello World Agent
=================

Starting hello-agent...

Hello! I'm here to help. What can I do for you?
```

The exact wording may vary, but you should get a friendly greeting response.

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
