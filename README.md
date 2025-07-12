# Arkavo Edge

Instant, secure orchestration for AI agents—launch, mesh, and monitor in real time.

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

## Quick Start
```bash
# Install (macOS arm64):
curl -L https://github.com/arkavo-org/arkavo-edge/releases/latest/download/arkavo-macos-aarch64.tar.gz | tar -xz
sudo mv arkavo /usr/local/bin

# Launch UI & first agent
arkavo ui &
arkavo agent init my-first-agent && arkavo agent run
```
