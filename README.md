# Arkavo Edge

Instant, secure orchestration for AI agents—launch, mesh, and monitor in real time.

## Quick Start
```bash
# Install (macOS arm64):
curl -L https://github.com/arkavo-org/arkavo-edge/releases/latest/download/arkavo-macos-aarch64.tar.gz | tar -xz
sudo mv arkavo /usr/local/bin

# Launch agent (auto-configures on first run)
arkavo

# Launch web UI
arkavo ui
```

## Why Arkavo?
- **True zero-config discovery:** agents auto-find each other with mDNS/DNS-SRV.
- **Built for performance:** Rust core pushes ≤ 2 ms A2A round-trips on commodity Macs.
- **Visual flow map:** instant insight into who’s talking to whom

## Key Features
| Feature                       | What you get                                                              |
|-------------------------------|---------------------------------------------------------------------------|
| **Agent Orchestration UI**    | Web + TUI dashboards that animate live data-flows.                        |
| **Plug-in Core**              | Drop-in providers (Ollama, OpenAI, Anthropic, …) with cost-aware routing. |
| **Cross-platform automation** | Unified iOS simulator control for mobile QA.                              |

### Auto-Configuration

When you run `arkavo` for the first time in a directory, it automatically:
- Creates an `AGENTS.md` configuration file
- Creates a `.arkavo` storage directory
- Generates a unique agent ID based on your directory name (e.g., `myproject-a1b2c3d`)
- Configures default settings for immediate use
- Starts the agent with mDNS discovery enabled
