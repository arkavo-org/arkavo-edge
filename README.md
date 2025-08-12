# Arkavo Edge

Instant, secure orchestration for AI agents—launch, mesh, and monitor in real time.

## Quick Start

### Install via Homebrew (macOS/Linux)
```bash
brew tap arkavo-org/homebrew-arkavo
brew install arkavo
```

### Install on Windows
Download the latest Windows binary from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases) and add it to your PATH:
```powershell
# Download and extract arkavo.exe to a directory in your PATH
# For example, to C:\Program Files\arkavo\
```

### Launch
```bash
# Launch agent (auto-configures on first run)
arkavo

# Launch web UI
arkavo ui
```

## Why Arkavo?
- **True zero-config discovery:** agents auto-find each other with mDNS/DNS-SRV (optional feature).
- **Built for performance:** Rust core pushes ≤ 2 ms A2A round-trips on commodity Macs.
- **Visual flow map:** instant insight into who's talking to whom

## Key Features
| Feature                       | What you get                                                              |
|-------------------------------|---------------------------------------------------------------------------|
| **Agent Orchestration UI**    | Web + TUI dashboards that animate live data-flows.                        |
| **Plug-in Core**              | Drop-in providers (Ollama, OpenAI, Anthropic, Kimi, …) with cost-aware routing. |
| **Cross-platform automation** | Unified iOS simulator control for mobile QA (macOS only).                 |

### Auto-Configuration

When you run `arkavo` for the first time in a directory, it automatically:
- Creates an `AGENTS.md` configuration file
- Creates a `.arkavo` storage directory
- Generates a unique agent ID based on your directory name (e.g., `myproject-a1b2c3d`)
- Configures default settings for immediate use
- Starts the agent with optional mDNS discovery (if available)

### Kimi Integration

Arkavo Edge now supports Kimi (Moonshot AI) models including the 128k context window variant:
- Configure agents with `model: kimi://moonshot-v1-128k` in AGENTS.md
- Add `MOONSHOT_API_KEY: sk-your-api-key` to the agent configuration in AGENTS.md
- API keys are securely disseminated from the UI orchestrator to agents
- Supports 8k, 32k, and 128k context models

### OpenTDF Authorization

Arkavo Edge integrates with [OpenTDF](https://opentdf.io) platform for entitlement-based access control:
- **Fine-grained permissions:** Control MCP tool execution with attribute-based policies
- **JWT-based authentication:** Secure token validation via Entity Resolution Service v2
- **Connect protocol support:** Uses OpenTDF Authorization v2 APIs with efficient HTTP/JSON
- **Smart caching:** Reduces latency with TTL-aware decision caching
- **Fail-closed security:** Denies access by default with safe diagnostic tool allowlist

Configure with environment variables:
- `OPENTDF_BASE_URL`: Platform endpoint (default: https://platform.opentdf.io)
- `OIDC_ISSUER`: Token issuer for validation
- `AUD`: Expected audience claim

## Platform Support

| Platform | Architecture | Features |
|----------|-------------|----------|
| macOS    | ARM64 (Apple Silicon) | Full support including iOS testing, local/remote LLM, mDNS* |
| Linux    | x86_64, ARM64 | Full support with local/remote LLM, mDNS* |
| Linux (musl) | x86_64 | Static binary with memory and remote LLM support |
| Windows  | x86_64 | Memory and remote LLM support (no iOS testing) |

*mDNS discovery is now optional and uses pure Rust implementation (no system dependencies)

**Note:** iOS simulator automation and testing capabilities are only available on macOS.

## Building from Source

### Default Build (without mDNS)
```bash
cargo build --release
```

### Build with mDNS Discovery
```bash
cargo build --release --features default-with-mdns
```

The mDNS feature now uses a pure Rust implementation (`mdns-sd` crate) that doesn't require system libraries like Avahi or Bonjour. This makes builds more portable and simplifies deployment across different platforms.
