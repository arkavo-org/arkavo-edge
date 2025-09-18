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
- **True zero-config discovery:** agents auto-find each other with mDNS/DNS-SRV.
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
- Starts the agent with mDNS discovery enabled

### Kimi Integration

Arkavo Edge now supports Kimi (Moonshot AI) models including the 128k context window variant:
- Configure agents with `model: kimi://moonshot-v1-128k` in AGENTS.md
- Add `MOONSHOT_API_KEY: sk-your-api-key` to the agent configuration in AGENTS.md
- API keys are securely disseminated from the UI orchestrator to agents
- Supports 8k, 32k, and 128k context models

### DeepSeek Integration

Arkavo Edge provides first-class support for DeepSeek's API with both standard and reasoning models:
- **Anthropic-compatible API**: Full support for DeepSeek's Anthropic-style endpoints
- **Function Calling**: Support for up to 128 tools per request with automatic schema validation
- **Model Selection**: Use `deepseek-chat` (default) or `deepseek-reasoner` via `DEEPSEEK_MODEL` environment variable
- **Strict Mode (Beta)**: Enhanced reliability with JSON Schema validation for tool arguments
- **Automatic Fallback**: Seamlessly switches from `deepseek-reasoner` to `deepseek-chat` when tools are present

Configure with environment variables:
- `DEEPSEEK_API_KEY`: Your DeepSeek API key
- `DEEPSEEK_MODEL`: Model to use (`deepseek-chat` or `deepseek-reasoner`)
- `DEEPSEEK_BASE_URL`: Optional custom API endpoint

Usage:
```bash
# Interactive chat with DeepSeek
DEEPSEEK_API_KEY=your-key arkavo chat --model deepseek

# Single prompt
DEEPSEEK_API_KEY=your-key arkavo chat --model deepseek --prompt "Explain quantum computing"

# With reasoning model
DEEPSEEK_API_KEY=your-key DEEPSEEK_MODEL=deepseek-reasoner arkavo chat --model deepseek --prompt "Solve: 15 * 23"
```

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
| macOS    | ARM64 (Apple Silicon) | Full support including iOS testing, local/remote LLM, mDNS |
| Linux    | x86_64, ARM64 | Full support with local/remote LLM, mDNS |
| Linux (musl) | x86_64 | Static/slim binary with memory and mDNS support |
| Windows  | x86_64 | Memory, remote LLM, and mDNS support (no iOS testing) |

mDNS discovery uses pure Rust implementation (mdns-sd crate) with no system dependencies

**Note:** iOS simulator automation and testing capabilities are only available on macOS.

## Building from Source

```bash
cargo build --release
```

The default build includes mDNS discovery using a pure Rust implementation (`mdns-sd` crate) that doesn't require system libraries like Avahi or Bonjour. This provides true portability across all platforms.

## Local Development Environment

- **Rust toolchain:** Install the latest stable toolchain with `rustup` and add `rustfmt`/`clippy` components (`rustup component add rustfmt clippy`).
- **Clone and bootstrap:**
  ```bash
  git clone https://github.com/arkavo-org/arkavo-edge.git
  cd arkavo-edge
  git submodule update --init --recursive   # pulls vendor/llama.cpp for llama.cpp builds
  ```
- **Optional iOS tooling:** macOS builds embed Meta’s idb companion for simulator automation. Download the prebuilt bundle and reuse it locally:
  ```bash
  mkdir -p vendor/idb-prebuilt
  curl -L https://github.com/arkavo-org/idb/releases/download/1.4.0-arkavo/idb_companion-1.4.0-arkavo-macos-arm64.tar.gz \
    -o vendor/idb-prebuilt/idb_companion-1.4.0-macos-arm64.tar.gz
  curl -L https://github.com/arkavo-org/idb/releases/download/1.4.0-arkavo/idb_companion-1.4.0-arkavo-macos-arm64.tar.gz.sha256 \
    -o vendor/idb-prebuilt/idb_companion-1.4.0-macos-arm64.tar.gz.sha256
  (cd vendor/idb-prebuilt && shasum -c idb_companion-1.4.0-macos-arm64.tar.gz.sha256)
  tar -xzf vendor/idb-prebuilt/idb_companion-1.4.0-macos-arm64.tar.gz -C vendor/idb-prebuilt
  ```
  Then export `ARKAVO_IDB_VENDOR_DIR=$(pwd)/vendor/idb-prebuilt` (or `ARKAVO_SKIP_IDB_DOWNLOAD=1` with the same directory) before building so `build.rs` uses the local artifacts.
- **Workspace diagnostics:**
  ```bash
  cargo fmt
  cargo check --workspace
  cargo clippy --workspace --all-features
  ```
- **Lightweight checks without optional vendors:** When Metal/llama.cpp or idb assets are unavailable (CI sandboxes, e.g.), you can focus on core crates:
  ```bash
  cargo check -p arkavo-terminal
  cargo check -p arkavo-protocol --features metrics
  cargo check --workspace --exclude arkavo-mcp-macos --exclude arkavo-llama-cpp-sys
  ```

### Useful Developer Commands

- Validate release sizes locally: `cargo xtask check-binary-size --limit-mb 60 --package arkavo`
- Profile diff rendering: `cargo bench -p arkavo-terminal diff_render`
- Inspect performance telemetry in the TUI by running `arkavo terminal` and watching the status bar for diff and router latency budgets.
