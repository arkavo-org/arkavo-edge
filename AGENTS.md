# Arkavo Edge Agent Guidelines

**Context**: You are working on Arkavo Edge, an agentic CLI for AI-driven code transformations.
**Goal**: Production-grade, secure, efficient, zero-config software.

## Core Development Rules
- **Production Quality**: No stubs, mocks, placeholders, or "demo" code. Implement fully.
- **Modular Architecture**:
  - One crate per capability (flat `crates/` structure).
  - Naming: General to specific.
  - Minimal dependencies (prefer `std`).
  - Clear interfaces between components.
- **Code Standards**:
  - **Size**: Implementation code (excluding `#[cfg(test)]` modules) should stay under 400 lines. Split when a file has multiple distinct responsibilities, not to hit a line count.
  - **Style**: `cargo fmt` required. No dead code (`#[allow(dead_code)]` forbidden).
  - **Comments**: Explain *why*, not *what*. No TODOs or status tracking.
  - **Safety**: No hardcoded responses. LLM handles generation.
- **Documentation**:
  - No numbered headings in Markdown.
- **Performance**:
  - Router response ≤ 50ms.
  - Binary ≤ 65MB.
  - No `--release` builds during development (use debug).

## Testing & Quality
- **Requirement**: ≥85% coverage. No clippy warnings.
- **Regression**: Every bug fix **MUST** have a regression test.
- **Structure**:
  - Unit: Inline `#[cfg(test)]` modules.
  - Integration: `tests/` directory in crate root.
- **Commands**:
  - Test: `cargo nextest run` (preferred) or `cargo test`.
  - Lint: `cargo clippy -- -D warnings`.

## Pre-Push Checklist
Before pushing to remote, run these checks:

```bash
# 1. Format check
cargo fmt -- --check

# 2. Build check (catches compilation errors)
cargo build -q

# 3. Lint check (catches style issues)
cargo clippy -- -D warnings

# 4. Security vulnerability tests
## Unit tests for security fixes
cargo test -p arkavo-protocol --test security_vulnerabilities

## Mock provider unit tests
cargo test -p arkavo-cli --lib mock_provider::

## E2E DLP/PII leak detection tests
./tests/e2e_security_test.sh

## CLI security tests with local models
./tests/security_cli_test.sh

## DLP/PII policy tests
./tests/dlp_pii_security_test.sh
```

**Security Test Failure Protocol**:
- If `e2e_security_test.sh` shows PII leaks, DLP scrubbing is not working
- If `security_vulnerabilities` tests fail, security fixes are broken
- If `mock_provider` tests fail, PII detection patterns need updating
- **Never push with security test failures**

## Architecture & Tech Stack
- **Cross-Platform**: macOS (arm64), Linux (x64/aarch64), Windows (x86_64).
- **Security**:
  - **NO OpenSSL**: Use `rustls` exclusively (musl compatibility).
  - **Secrets**: Never commit or write to docs any API keys.
- **Tool Pattern**: Each crate exports its own LLM tools via `register_tools(registry)`. No central tool crate. Avoids circular dependencies.
- **Windows Specifics**:
  - Default build excludes C++ (llama-cpp) to avoid MSVC issues.
  - Ensure new deps work without C++ on Windows.

## Git & Workflow
- **Branching**: 
  - `feature/<name>`. No release branches.
  - `fix/<name>`. bug fixes only.
  - `main` is protected with CI checks.
- **Commits**:
  - **NO Conventional Commits** (e.g., avoid `feat:`, `fix:`).
  - Bump semver in `Cargo.toml` on feature completion. Every PR, including each stack layer, bumps above the one below it: every merge to `main` is a release, and CI rejects a version that isn't higher than `main`'s.
  - Commit `Cargo.lock` whenever `Cargo.toml` changes.
- **PRs**:
  - **Short titles** (≤ 60 chars). One topic per PR — do not join unrelated changes with `+`, `&`, or `and`. Use the body for detail.
  - No changelog files (GitHub handles it).
  - **Stack from creation, or don't split.** Several topics from one piece of work, or work that builds on an open PR, start as a GitHub stack (`gh stack init`, then `gh stack add` per topic), each layer based on the one below. Never open a run of standalone PRs to restack later: restacking rewrites branches other sessions own, and standalone PRs off the same `main` all conflict on the version bump.
  - **Incidental fixes ride along.** A flaky test, stale default or typo found mid-task becomes a layer of the current stack, not a new PR.
  - **Check the queue before opening.** Run `gh pr list` first. If an open PR touches the same crates, or the new work needs it, add a layer to that stack instead of opening beside it.
  - **Keep the ready queue short**: at most 3 ready-for-review PRs or stacks open at once. At the limit, merge, fix or close one before marking another ready. A ready PR that stops merging cleanly is fixed the same day or moved back to draft.
  - **Rebasing a stack:** `gh stack rebase`, then re-bump each layer above the one below it when `main` has moved. Merge the stack whole with `gh stack merge`.
- **Docs**: Technical docs in `docs/`. 

## Agent Configuration
- **Autonomous**: Auto-detect capabilities.  No manual configuration.
- **Orchestration**: Optional centralized control plane.

## Local Model Support

### Local edge models
First run auto-downloads Gemma 4 sized to the device (E2B + 12B on desktop/workstation; E4B on a Raspberry Pi 5). Ministral 3B/8B (Mistral's edge models, with vision) are also supported:
- **Ministral 3B**: Raspberry Pi 5 / low-memory, 8GB RAM minimum
- **Ministral 8B**: Desktop/laptop, 12GB VRAM recommended

Local models auto-download from HuggingFace on first use. The canonical model registry is `ModelChoice` in `crates/arkavo-router/src/decision.rs`.

### Reasoning Mode
For complex tasks requiring step-by-step thinking, use Ministral Reasoning variants or add a system prompt:
```
Think through this problem step-by-step before providing your answer.
```

## 6. Useful Commands

# Build (Debug). No release builds.
```bash
cargo build -q
```

# Test
```bash
cargo test
```

# Run
```bash
ARKAVO_DEBUG=1 ARKAVO_DEBUG_CHAT=1 cargo run -p arkavo -- chat --prompt "What time is it?"
```

## 7. Environment Variables
- `ARKAVO_DEBUG=1`: General debug logging.
- `ARKAVO_DEBUG_CHAT=1`: Chat/Template/Token debug.
- `ARKAVO_DELEGATION_PUBLIC_KEY_PEM`: Trusted authnz-rs ES256 public key (inline PEM or path to PEM file) used to verify delegation JWTs at registration. Unset → delegation entitlements are never granted (fail-closed).
- `ARKAVO_ALLOW_UNVERIFIED_DELEGATION=1`: INSECURE dev/test escape hatch — accepts delegation JWTs without signature verification. Never set in production.
- `ARKAVO_CHAT_TIMEOUT_SECS`: Time budget for a chat inference, whole seconds from 5 to 86400. Unset → a named local model gets 180 seconds.
- `ARKAVO_DISABLE_SPEC_DECODING`: Any value other than empty, `0` or `false` turns speculative decoding off. For diagnosing corrupted output.
- `ARKAVO_MAX_CONTEXTS`: Inference contexts kept per local model in one process. Unset → 1; further requests wait.
- `ARKAVO_CONTEXT_WAIT_SECS`: How long a request waits for a free context before it fails. Unset → 300.
- `ARKAVO_TOOL_ENV_PASSTHROUGH=NAME,NAME`: credential-shaped variables built-in tools may still pass to the programs they run (e.g. a private registry token for `cargo test`); every other `*_KEY`/`*_TOKEN`/`*_SECRET`-style variable is withheld from tool subprocesses.
- `ARKAVO_ALLOW_UNSANDBOXED_BROWSER=1`: INSECURE — lets `browser_cdp` start Chrome without its sandbox on a Linux host that has no usable one (user namespaces disabled). Running as root on Linux disables it automatically, with a warning; macOS and Windows always sandbox.
- ccache must be installed for development builds