# Arkavo Edge

Instant, secure orchestration for AI agents—launch, mesh, and monitor in real time.

## Quick Start

### Install on macOS

Download the installer from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases), open the .pkg file, and follow the installation wizard.

**For advanced users:** Install via Homebrew
```bash
brew tap arkavo-org/homebrew-arkavo
brew install arkavo
```

### Install on Linux
```bash
brew tap arkavo-org/homebrew-arkavo
brew install arkavo
```

**Raspberry Pi 5:** Download ARM64 binary from [releases](https://github.com/arkavo-org/arkavo-edge/releases). See [deployment guide](docs/raspberry-pi-deployment.md) for setup. Recommended models: Ministral 3B (text+vision) or gemma-3 270M.

### Install on Windows
Download the installer from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases) and run the .exe file.

### Launch
```bash
# Start an agent (zero config)
arkavo

# Or launch web UI
arkavo ui
```

That's it. No configuration files, no setup. Agents auto-discover via mDNS and form a mesh.

## Coming from OpenClaw?

See the [migration guide](docs/openclaw-migration-guide.md) for a full comparison: what you gain (budget controls, TDF encryption, PII preflight, offline operation), what's different, and step-by-step setup.

## Why Arkavo?
- **Zero config:** Just run `arkavo`. Auto-naming, auto-routing, auto-discovery.
- **Fast:** Low-latency agent-to-agent communication. Run `cargo bench -p arkavo-protocol --bench a2a_latency` to measure.
- **Visual:** See live agent communication flows in real-time.

## Features
- Multi-provider routing (OpenAI, Anthropic, Gemini, Kimi, DeepSeek, local models)
- **Ministral 3 support** - Local edge models (3B/8B/14B) with vision via llama.cpp
- Cost-aware model selection
- GitHub issue orchestration
- iOS simulator automation (macOS only)
- Security scanning (Semgrep, OSV, SBOM)

## Usage Examples

### Chat
```bash
# Use any provider with API key
GEMINI_API_KEY=your-key arkavo chat --prompt "Hello"
DEEPSEEK_API_KEY=your-key arkavo chat --prompt "Explain Rust"
```

### Context Control Demo
The Autonomous Refactor demo demonstrates Active Context Management. It simulates a large-scale "breaking change" refactor that generates extensive compiler output, showing how the Context Ledger maintains a small active window while preserving data access.

```bash
cd examples/autonomous_refactor
./run_demo.sh
```

### Custom Agent Config (Optional)
```bash
arkavo agent init my-agent  # Creates AGENTS.md template
# Edit AGENTS.md to set model, capabilities, API keys
arkavo  # Runs with your config
```

### Security (Optional)

**OpenTDF Integration:** Fine-grained access control for MCP tools via [OpenTDF](https://opentdf.io). Set `OPENTDF_BASE_URL`, `OIDC_ISSUER`, and `AUD` environment variables.

## Coding Agent Toolset

Arkavo Edge includes a comprehensive suite of MCP tools for AI coding agents:

### Code Search & Intelligence
- **codegrep_search**: Fast repository-wide code search with ripgrep
- **struct_find_replace**: Language-aware structural search and replace with Comby
- **syntax_tree**: AST parsing for syntax-aware code analysis with tree-sitter

### Security & Quality
- **sec_semgrep**: SAST scanning with Semgrep
- **deps_osv**: Dependency vulnerability scanning with OSV-Scanner
- **sbom_syft**: SBOM generation with Syft

### Test & Automation
- **browser_cdp**: Chrome DevTools Protocol automation via chromiumoxide
- **test_run**: Multi-language test runner (pytest, jest, go test, cargo test, xcodebuild)

### GitHub Orchestration
```bash
# Auto-handle GitHub issues with AI agents
GITHUB_TOKEN=ghp_token arkavo orchestrator poll --repo owner/repo
```

Features: Issue classification, agent assignment, PR reviews, budget tracking. See [orchestrator docs](crates/arkavo-orchestrator/README.md).

### Ephemeral Workspaces
- **workspace_container**: Container-based isolated execution with resource quotas (Docker/Podman)

### SWE-bench Evaluation
- **swe_bench**: Objective benchmarking harness with metrics tracking

See [docs/coding-agent-toolset.md](docs/coding-agent-toolset.md) for complete tool documentation.

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

### Prerequisites

Install required build tools:

```bash
# macOS
brew install cmake ccache

# Linux (Debian/Ubuntu)
sudo apt install cmake ccache build-essential

# Linux (Fedora)
sudo dnf install cmake ccache gcc-c++
```

### Setup llama.cpp

Clone the llama.cpp dependency (not tracked in git):

```bash
git clone https://github.com/ggerganov/llama.cpp vendor/llama.cpp
cd vendor/llama.cpp
git checkout ecf74a841
cd ../..
```

### Build

```bash
cargo build
```

The default build includes mDNS discovery using a pure Rust implementation (`mdns-sd` crate) that doesn't require system libraries like Avahi or Bonjour. This provides true portability across all platforms.
