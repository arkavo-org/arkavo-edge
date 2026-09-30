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

**Debian/Ubuntu (x86_64):** Download `arkavo_<version>_amd64.deb` from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases) and install it:

```bash
sudo apt install ./arkavo_<version>_amd64.deb
```

**Other distributions:** Download `arkavo-<version>-x86_64-linux.tar.gz` from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases), extract it, and place `arkavo` on your `PATH`.

**Raspberry Pi 5:** Download ARM64 binary from [releases](https://github.com/arkavo-org/arkavo-edge/releases). See [deployment guide](docs/raspberry-pi-deployment.md) for setup. First run auto-selects an edge model for the device (Pi 5 → Gemma 4 E4B).

### Install on Windows
Download `arkavo-<version>-x86_64-windows.zip` from the [releases page](https://github.com/arkavo-org/arkavo-edge/releases), extract `arkavo.exe`, and add its folder to your `PATH`. There is no installer. This build has no local inference backend; see the note under [Platform Support](#platform-support).

### Launch
```bash
# Start an agent (zero config)
arkavo

# Or launch web UI
arkavo ui
```

That's it. No configuration files, no setup. Agents on the same machine and on other devices on the local network auto-discover via mDNS and form a mesh.

By default an agent is discoverable and reachable on the local network: it listens on every interface and announces itself over mDNS. Its RPC endpoint is not authenticated yet, so run it on networks you trust. The agent says so on stderr each time it starts this way.

To keep an agent on this machine, start it with `--bind 127.0.0.1`. It then listens on loopback and is not announced on the network; agents on the same machine still discover it.

```bash
arkavo --bind 127.0.0.1
```

`--bind` takes an IP address, with a port if you want one (`--bind 127.0.0.1:8343`, `--bind [::1]`); without a port, `-p` or the kit's port applies. A kit can pin an address with `runtime.listen`:

```yaml
runtime:
  listen: "127.0.0.1:8343"   # this machine only, fixed port
```

`--bind` overrides the kit's address and says so. `arkavo security audit` reports the address an agent would listen on in the current directory; add `--bind <address>` to audit an agent started with it.

On first run, Arkavo downloads two local models sized to your device — a small model for fast routing (Gemma 4 E2B) and a larger model for inference (Gemma 4 12B on desktop/workstation; Gemma 4 E4B on a Raspberry Pi 5).

### Trusting an agent

To authorize an agent from another device, show its identity QR — the agent's `DID:key`, entitlements and RPC endpoint:

```bash
arkavo agent run --trust   # or simply: arkavo --trust
```

`--trust` changes nothing about where the agent listens: by default the endpoint in the QR code is the agent's address on the local network, which the device that scans it can reach. That endpoint is not authenticated yet, so show the code on a network you trust. With `--bind 127.0.0.1` the endpoint in the code is loopback, and other devices cannot connect to it.

## Coming from OpenClaw?

See the [migration guide](docs/openclaw-migration-guide.md) for a full comparison: what you gain (budget controls, TDF encryption, PII preflight, offline operation), what's different, and step-by-step setup.

## Why Arkavo?
- **Zero config:** Just run `arkavo`. Auto-naming, auto-routing, and auto-discovery between agents on the same machine and across devices on the local network. The RPC endpoint is not authenticated yet, so run agents on networks you trust; `--bind 127.0.0.1` keeps an agent on this machine (see [Launch](#launch)).
- **Fast:** Low-latency agent-to-agent communication (benchmarkable from source — see [Building from Source](#building-from-source)).
- **Visual:** See live agent communication flows in real-time.

## SwarmKit

Declarative multi-agent kits where each role declares its own TDF Attribute Release Policy. The orchestrator constructs role-scoped policies before any data reaches the role — push the trust boundary inward.

```bash
# Check a kit manifest
arkavo kit validate examples/code-review-kit/code-review-kit.swarmkit.yaml

# Load a kit into the web UI gateway; each role appears in the AG-UI ARP panel
ARKAVO_SWARMKIT_PATH=examples/code-review-kit/code-review-kit.swarmkit.yaml arkavo ui

# Run a role of the kit as an agent (one process per role)
arkavo agent -c examples/code-review-kit/code-review-kit.swarmkit.yaml -n reviewer -p 8343
```

`ARKAVO_SWARMKIT_PATH` loads the kit into the gateway that `arkavo ui` starts in builds with the web renderer (Homebrew, the `.deb`, and the release archives). The macOS `.pkg` build opens a native window instead and does not start that gateway. Bare `arkavo` does not start it either: with the variable set it runs the kit's first role as a single agent.

Loading a kit builds per-role policy, trace, and panel entries. It does not start the roles or run the workflow between them; see [what a kit launch does today](docs/SWARMKIT.md#what-a-kit-launch-does-today).

Four example kits: `campaign-kit`, `code-review-kit`, `vrm-production-kit`, `compliance-kit`. They live in this repository's `examples/` directory and are not part of the Homebrew, package, or archive installs, so clone the repository to use them, or start your own kit with `arkavo kit init <name>`. Full guide: [docs/SWARMKIT.md](docs/SWARMKIT.md).

## Features
- **SwarmKit** - Declarative multi-agent kits with per-role TDF attribute-release policies. Four example kits in `examples/` covering marketing, code review, creative, and regulated domains. See [docs/SWARMKIT.md](docs/SWARMKIT.md).
- Multi-provider routing (OpenAI, Anthropic, Gemini, Kimi, DeepSeek, local models)
- **Local edge models via llama.cpp** - Gemma 4 (E2B/E4B/12B) by default; Ministral 3B/8B (with vision) also supported
- Cost-aware model selection (real per-token estimates on full macOS/Linux builds; the musl-slim and Windows binaries use an approximate estimator)
- iOS simulator automation (macOS only)
- Security scanning (Semgrep, OSV, SBOM)

## Commands

Run `arkavo <command> --help` for the options of `agent`, `chat`, `task`, `ui`, `kit`, `model`, and `security`.

| Command | What it does |
|---------|--------------|
| `arkavo`, `arkavo agent` | Run an agent. `-c <kit>` names a SwarmKit manifest (default: discover `.arkavo/*.swarmkit.yaml` or `./*.swarmkit.yaml`), `-n <role-id>` selects a role of a multi-role kit (default: the first role), `-p <port>` sets the listen port (default: a random free port), `-v` prints startup messages, `--bind <address>` sets the listen address (`127.0.0.1` keeps the agent on this machine; a port may follow, as in `127.0.0.1:8343`), `--trust` shows the authorization QR code. Without `--bind` or a `runtime.listen` in the kit, the agent listens on every interface. |
| `arkavo chat` | Interactive chat, or a one-shot query with `--prompt`. `--model <name or .gguf path>` picks the model; `--agent-id <id>` talks to a mesh agent. |
| `arkavo task` | Plan and apply code changes: `arkavo task 'fix all warnings'`. `--local-only` and `--mesh-only` choose where the task runs, `--agent-id <id>` targets one agent. Without a task it commits existing changes (`-y`, `-m <message>`, `--push`, `--no-validate`). |
| `arkavo ui` | Launch the web UI (default port 7700). |
| `arkavo kit init <name>` | Write a minimal single-role kit to `.arkavo/<name>.swarmkit.yaml`. |
| `arkavo kit validate <path>...` | Load and validate one or more kit files. |
| `arkavo kit migrate-from-agents-md --in <path> --out <path>` | Best-effort conversion of an AGENTS.md file into a kit. |
| `arkavo model` | Manage local models: `list`, `download [name]`, `add <path> --name <name>`, `protect <path>` (wrap a GGUF into a KAS-gated `.gguf.tdf` archive). `switch` is accepted but not implemented. |
| `arkavo mcp proxy` | Permit-gated stdio MCP relay in front of an upstream MCP server: `arkavo mcp proxy --policy-bundle-hash <64 hex> --issuer-key <hex> [--hash sha256\|blake3] -- <upstream command> [args...]`. A `tools/call` is admitted only with a valid permit and proof of possession. |
| `arkavo security audit` | Report on the configuration an agent would run with in the current directory, including the address its RPC endpoint would listen on. `--bind <address>` audits the agent as started with `--bind <address>`, `--json` prints JSON. Exits with status 1 when a check fails. |
| `arkavo login`, `arkavo logout` | Sign in with Arkavo Creator; clear the stored identity token. Both act immediately and take no options. |

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
arkavo kit init my-agent  # Writes .arkavo/my-agent.swarmkit.yaml
# Edit the kit to set model, skills — API keys stay in env vars, never the kit
arkavo agent  # Runs with your config
```

### Environment Variables

| Variable | Effect |
|----------|--------|
| `ARKAVO_DEBUG=1` | General debug logging. |
| `ARKAVO_DEBUG_CHAT=1` | Chat, template, and token debug output. |
| `ARKAVO_SWARMKIT_PATH` | Path of the kit to use instead of discovering one in the working directory. It is also the kit the `arkavo ui` gateway loads. |
| `ARKAVO_SWARMKIT_VERIFY=required` | Enforce skill signature verification when the gateway loads a kit. |
| `ARKAVO_SKIP_FIRST_RUN=1` | Never prompt for the first-run model download. Commands that need a local model still fail until one is provisioned. |
| `ARKAVO_CHAT_TIMEOUT_SECS` | Time budget for a chat inference, in whole seconds from 5 to 86400. Without it, a named local model gets 180 seconds. |
| `ARKAVO_DISABLE_SPEC_DECODING` | Any value other than empty, `0`, or `false` turns speculative decoding off. Use it to diagnose corrupted output. |
| `ARKAVO_N_CTX` | Context window for local models, in tokens. Without it, a model gets a quarter of its trained context, up to 16384. |
| `ARKAVO_MAX_CONTEXTS` | Inference contexts kept per local model in one process. The default is 1; further requests wait for it. |
| `ARKAVO_CONTEXT_WAIT_SECS` | How long a request waits for a free context before it fails. The default is 300. |
| `GGML_METAL_RESIDENCY_KEEP_ALIVE_S` | On macOS, seconds an agent keeps its model's GPU memory wired after an inference. Unset, an agent releases it at once so other agents on the machine can run; set it (llama.cpp's own default is 180) when one agent has the GPU to itself. |

API keys for cloud providers are read from the environment (for example `GEMINI_API_KEY`) and are never written in a kit.

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
| **ABAC / attribute release policies** | Library shipped, not wired into kit launch | Roles declare TDF Attribute Release Policies and kit validation checks them. The runtime library turns each one into a role-scoped OpenTDF policy (`role_policy()`), but the `arkavo ui` and `arkavo agent` launch paths do not call it yet. |
| **SwarmKit policy isolation** | Shipped for kits loaded into the gateway | A kit loaded through `ARKAVO_SWARMKIT_PATH` gets one Agent Runtime Policy, policy cache, and decision trace per role. On the `arkavo agent -c <kit>` path a role's `isolation`, network egress, budget, and `mcp_tools` grant fields are parsed but not enforced: the process takes the role's id, model, and skill instructions, plus the kit-level `runtime` block. |
| **DID:key identity** | Shipped | Agents are identified by `did:key` derived from an Ed25519 keypair; identity is stable per device. |
| **mDNS mesh discovery** | Shipped | Pure-Rust mDNS with no system Avahi/Bonjour dependency; agents auto-discover and form a local mesh. By default an agent announces itself on the local network and listens on every interface. With `--bind 127.0.0.1` it listens on loopback and is announced on the loopback interface only. |
| **Local inference** | Shipped | Gemma 4 and Ministral models run via llama.cpp on the local device; no cloud required for routing or inference. |
| **DLP / PII preflight** | Shipped, off by default | Preflight policies run before any model inference and refuse a matching request, reporting the policy id and reason. They are active only when the kit declares `runtime.preflight`. Preflight blocks; it does not redact or rewrite the request. |
| **PII leak regression tests** | Shipped | `tests/e2e_security_test.sh`, `tests/security_cli_test.sh`, `tests/dlp_pii_security_test.sh`. |

### Roadmap / not yet landed

| Capability | Status | Notes |
|------------|--------|-------|
| **RPC endpoint authentication** | Not yet | The agent's RPC endpoint does not authenticate callers, and it is served without TLS. By default it is reachable from the local network, so run agents on networks you trust. `--bind 127.0.0.1`, or a loopback `runtime.listen` in the kit, keeps the endpoint on this machine. The methods that read and replace the kit are served only on a loopback endpoint. |
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

**Note:** The Linux (musl) and Windows builds are compiled without a local inference backend. `arkavo`, `arkavo agent`, `arkavo chat`, `arkavo task`, and `arkavo ui` require one and exit with an error on those builds; utility commands such as `arkavo kit` still run. A cloud API key does not replace the local backend.

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
git checkout f280b26983ad0fdb705a0d9ebf0503e76f2899b0  # b10615
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

# Validate a SwarmKit manifest without the installed binary
# (the installed equivalent is `arkavo kit validate <path>`)
cargo run -p arkavo-swarmkit --example validate_kit -- \
  examples/compliance-kit/compliance-kit.swarmkit.yaml
```
