# Arkavo Edge

Instant, secure orchestration for AI agents—launch, mesh, and monitor in real time.

## Quick Start

### Install on macOS

Download the installer from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases), open the .pkg file, and follow the installation wizard.

**For advanced users:** Install via Homebrew
```bash
brew tap arkavo-org/homebrew-arkavo
brew trust --formula arkavo-org/arkavo/arkavo  # required on Homebrew 5.2+
brew install arkavo
```

### Install on Linux
```bash
brew tap arkavo-org/homebrew-arkavo
brew trust --formula arkavo-org/arkavo/arkavo  # required on Homebrew 5.2+
brew install arkavo
```

**Raspberry Pi 5:** Download ARM64 binary from [releases](https://github.com/arkavo-org/arkavo-edge/releases). See [deployment guide](docs/raspberry-pi-deployment.md) for setup. First run auto-selects an edge model for the device (Pi 5 → Gemma 4 E4B).

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

On first run, Arkavo downloads two local models sized to your device — a small model for fast routing (Gemma 4 E2B) and a larger model for inference (Gemma 4 12B on desktop/workstation; Gemma 4 E4B on a Raspberry Pi 5).

### Trusting an agent

To authorize an agent (e.g. from another device), show its identity QR — the agent's `DID:key` and entitlements:

```bash
arkavo agent run --trust   # or simply: arkavo --trust
```

## Coming from OpenClaw?

See the [migration guide](docs/openclaw-migration-guide.md) for a full comparison: what you gain (budget controls, TDF encryption, PII preflight, offline operation), what's different, and step-by-step setup.

## Why Arkavo?
- **Zero config:** Just run `arkavo`. Auto-naming, auto-routing, auto-discovery.
- **Fast:** Low-latency agent-to-agent communication (benchmarkable from source — see [Building from Source](#building-from-source)).
- **Visual:** See live agent communication flows in real-time.

## SwarmKit

Declarative multi-agent kits where each role declares its own TDF Attribute Release Policy. The orchestrator constructs role-scoped policies before any data reaches the role — push the trust boundary inward.

```bash
# Launch any kit at gateway boot
ARKAVO_SWARMKIT_PATH=examples/code-review-kit/code-review-kit.swarmkit.yaml arkavo
```

Four shipped kits: `campaign-kit`, `code-review-kit`, `vrm-production-kit`, `compliance-kit`. Full guide: [docs/SWARMKIT.md](docs/SWARMKIT.md). To validate a kit manifest from source, see [Building from Source](#building-from-source).

## Features
- **SwarmKit** - Declarative multi-agent kits with per-role TDF attribute-release policies. Four shipped examples covering marketing, code review, creative, and regulated domains. See [docs/SWARMKIT.md](docs/SWARMKIT.md).
- Multi-provider routing (OpenAI, Anthropic, Gemini, Kimi, DeepSeek, local models)
- **Local edge models via llama.cpp** - Gemma 4 (E2B/E4B/12B) by default; Ministral 3B/8B (with vision) also supported
- Cost-aware model selection (real per-token estimates on full macOS/Linux builds; the musl-slim and Windows binaries use an approximate estimator)
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

The agent uses these MCP tools in-process during `chat` and `task` — there's no separate server to start. Tools that shell out to an external binary register only when that binary is on `PATH` (noted below).

### Code Search & Intelligence
- **codegrep_search**: Fast repository-wide code search with ripgrep
- **struct_find_replace**: Language-aware structural search and replace with Comby
- **syntax_tree**: AST parsing for syntax-aware code analysis with tree-sitter

### Security & Quality (require the named binary on `PATH`)
- **sec_semgrep**: SAST scanning with Semgrep
- **deps_osv**: Dependency vulnerability scanning with OSV-Scanner
- **sbom_syft**: SBOM generation with Syft

### Test & Automation
- **browser_cdp**: Chrome DevTools Protocol automation via chromiumoxide
- **test_run**: Multi-language test runner (pytest, jest, go test, cargo test, xcodebuild)

### Ephemeral Workspaces (requires Docker/Podman)
- **workspace_container**: Container-based isolated execution with resource quotas

These nine tools are the ones the running agent can call (the binary also registers git, GitHub, web-search, shell, and TDF tools — see the full reference below).

### Benchmark harness

SWE-bench evaluation lives in the separate `arkavo-mcp-bench` crate, run from source — it is **not** a registered agent tool (it depends on the orchestration engine, which would form a dependency cycle if exposed through the tool registry).

See [docs/coding-agent-toolset.md](docs/coding-agent-toolset.md) for complete tool documentation.

## Security Status

For offensive-security reviewers: here's what is real today and what is still on the roadmap.

### Shipping now

| Capability | Status | Notes |
|------------|--------|-------|
| **OpenTDF / KAS encryption** | Shipped | Tool outputs and SwarmKit payloads can be wrapped in TDF; KAS policy enforcement is live. |
| **ABAC / attribute release policies** | Shipped | Roles declare TDF Attribute Release Policies; the orchestrator constructs role-scoped policies before data reaches the role. |
| **SwarmKit policy isolation** | Shipped | Each kit role gets its own policy envelope; no shared blanket entitlements. |
| **DID:key identity** | Shipped | Agents are identified by `did:key` derived from an Ed25519 keypair; identity is stable per device. |
| **mDNS mesh discovery** | Shipped | Pure-Rust mDNS with no system Avahi/Bonjour dependency; agents auto-discover and form a local mesh. |
| **Local inference** | Shipped | Gemma 4 and Ministral models run via llama.cpp on the local device; no cloud required for routing or inference. |
| **DLP / PII scrubbing** | Shipped | Pre-flight redaction of sensitive patterns before LLM context and provider calls. |
| **PII leak regression tests** | Shipped | `tests/e2e_security_test.sh`, `tests/security_cli_test.sh`, `tests/dlp_pii_security_test.sh`. |

### Roadmap / not yet landed

| Capability | Status | Notes |
|------------|--------|-------|
| **SEP / TPM hardware attestation** | In crate, not crypto-bound | `arkavo-attestation` detects the Secure Enclave on Apple Silicon and reports a security state, but the evidence is platform metadata, not a Secure-Enclave-signed quote. TPM backend is not implemented. |
| **Hardware-bound key storage** | Not yet | Device identity and agent keypairs are stored on disk with filesystem permissions; they are not yet stored in the Secure Enclave, Keychain (non-extractable), or a TPM. |
| **Verifiable remote attestation** | Not yet | Trust scoring currently treats identity as verified once a DID:key is known; there is no remote verification of attestation evidence yet. |

This split is intentional: encryption, access control, and identity are shipping now; hardware-bound trust roots are being built in the open.

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
git checkout ef570f63087b6a5a2930210a13f87990e8113927
cd ../..
```

### Build

```bash
cargo build
```

The default build includes mDNS discovery using a pure Rust implementation (`mdns-sd` crate) that doesn't require system libraries like Avahi or Bonjour. This provides true portability across all platforms.

### Development

These commands run against the source tree (not the installed binary):

```bash
# Measure agent-to-agent latency
cargo bench -p arkavo-protocol --bench a2a_latency

# Validate a SwarmKit manifest
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/compliance-kit/compliance-kit.swarmkit.yaml
```
