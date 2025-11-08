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

**Raspberry Pi 5:** Download ARM64 binary from [releases](https://github.com/arkavo-org/arkavo-edge/releases). See [deployment guide](docs/raspberry-pi-deployment.md) for setup with gemma-3 270M model.

### Install on Windows
Download the installer from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases) and run the .exe file.

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

### GitHub Integration
- **gh_checks**: GitHub Checks API integration with inline annotations
- **gh_pr_review**: PR reviews with line-level comments

### GitHub Issue Orchestration

Arkavo Edge can automatically handle GitHub issues through AI agents.

**Polling Mode** (recommended - no infrastructure required):
```bash
# Poll for new issues using GitHub personal access token
export GITHUB_TOKEN="ghp_your_token"
arkavo orchestrator poll --repo owner/repo --interval 300
```

**Webhook Server** (for teams with public infrastructure):
```bash
# Start webhook server
export ARKAVO_GITHUB_WEBHOOK_SECRET="your-webhook-secret"
export ARKAVO_GITHUB_APP_ID="123456"
export ARKAVO_GITHUB_APP_PRIVATE_KEY="$(cat private-key.pem)"

arkavo orchestrator start --port 3000
```

**Manual Processing** (for testing):
```bash
export GITHUB_TOKEN="ghp_your_token"
arkavo orchestrator process --repo owner/repo --issue 123
```

Features:
- **Multiple Modes**: Polling (95% of users), webhook server, or manual trigger
- **Intelligent Routing**: 4 execution strategies (AutoExecute, PlanFirst, OrchestratorConsultation, HumanApprovalRequired)
- **Issue Analysis**: Automatic classification (Bug, Feature, Docs) with complexity assessment
- **Agent Assignment**: Capability-based agent selection with load balancing
- **Progress Tracking**: Real-time status updates posted as issue comments
- **Budget Management**: Token budget tracking with automatic warnings

See [crates/arkavo-orchestrator/README.md](crates/arkavo-orchestrator/README.md) for complete orchestrator documentation.

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

```bash
cargo build --release
```

The default build includes mDNS discovery using a pure Rust implementation (`mdns-sd` crate) that doesn't require system libraries like Avahi or Bonjour. This provides true portability across all platforms.
