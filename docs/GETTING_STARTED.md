# Getting Started: Run Your First Arkavo Agent in 5 Minutes

This guide will walk you through setting up and running your first Arkavo agent, demonstrating the core strengths of the project: single binary distribution, simple text-based configuration, automatic discovery, and UI-based interaction.

## Prerequisites

- macOS (Apple Silicon) or Linux (x64/aarch64)
- No additional dependencies required - Arkavo is a single binary!

## Step 1: Install Arkavo

Run this command in your terminal for macOS (Apple Silicon):

```bash
curl -L https://github.com/arkavo-org/arkavo-edge/releases/download/v0.21.0-alpha/arkavo-macos-aarch64.tar.gz \
  | tar -xz
mv arkavo /usr/local/bin
```

For other platforms, check the [releases page](https://github.com/arkavo-org/arkavo-edge/releases).

## Step 2: Create Your First Agent

Initialize a new agent configuration:

```bash
arkavo agent init build-doc-bot
```

This creates an `AGENTS.md` file with a template configuration. The file uses a simple, readable format:

```markdown
# AGENTS.md

## build-doc-bot
purpose: Generate developer docs from repo README files
model:   ollama://127.0.0.1:11434/qwen:0.6b
listen:  0.0.0.0:8342
discovery:
  mdns: true
```

You can edit this file to customize your agent's purpose, model, and network settings.

## Step 3: Start the Agent

In your terminal, run:

```bash
arkavo agent run
```

You should see:
```
Starting agent: build-doc-bot
Purpose: Generate developer docs from repo README files
Model: ollama://127.0.0.1:11434/qwen:0.6b
Listen: 0.0.0.0:8342
Agent server started on 0.0.0.0:8342
mDNS service registered for agent: build-doc-bot
Press Ctrl+C to stop
```

## Step 4: Launch the UI

Open a **new terminal window** (keep the agent running!) and run:

```bash
arkavo ui
```

You'll see:
```
Starting Arkavo UI server on http://127.0.0.1:7700
Open this URL in your web browser
Press Ctrl+C to stop
```

## Step 5: Test the Agent

1. Open your web browser and navigate to **http://127.0.0.1:7700**
2. You should see your `build-doc-bot` agent appear as a card (it may take a few seconds for discovery)
3. Click on the agent card to open the interaction panel
4. The `promise_request` method will be pre-selected
5. In the `repo_url` field, enter: `https://github.com/arkavo-org/arkavo-edge`
6. Click **Send Request**
7. You'll receive a response with error code `-32601` and message "Method not implemented"

**This error is expected!** It confirms that:
- The agent is running correctly
- mDNS discovery is working
- The UI can communicate with the agent
- The entire pipeline is functional

## What's Next?

You've successfully:
- ✅ Installed Arkavo with zero dependencies
- ✅ Created an agent configuration with a simple text file
- ✅ Started an agent that broadcasts itself on the network
- ✅ Launched a web UI that discovers agents automatically
- ✅ Verified agent communication

### Next Steps

1. **Multiple Agents**: Add more agent configurations to your `AGENTS.md` file
2. **Custom Models**: Change the model URL to use different LLMs (OpenAI, Anthropic, local models)
3. **Network Configuration**: Adjust the `listen` address for different network setups
4. **Implementation**: As the A2A protocol is implemented, your agents will gain real capabilities

## Troubleshooting

### Agent doesn't appear in the UI
- Ensure both `arkavo agent run` and `arkavo ui` are running
- Check that `mdns: true` is set in your agent configuration
- Verify firewall settings allow mDNS (port 5353) and your agent's port

### "Method not implemented" error
- This is the expected result for this tutorial
- The A2A protocol implementation is ongoing
- Check GitHub for updates on method implementations

### Port already in use
- Change the port in `AGENTS.md` (e.g., `listen: 0.0.0.0:8343`)
- Or stop other services using the port

## Architecture Overview

What you just experienced demonstrates Arkavo's core architecture:

1. **Agent Configuration** (`AGENTS.md`): Simple, human-readable configuration
2. **A2A Protocol Server**: JSON-RPC server with mDNS broadcasting
3. **Web UI**: Discovers agents and provides interaction interface
4. **Zero Configuration**: No databases, no complex setup, just run and go

This simple flow scales to complex multi-agent systems while maintaining the same ease of use.