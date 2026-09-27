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
  - **Versioning:** feature and fix PRs never change the `Cargo.toml` version. A release is its own PR that only bumps `[workspace.package] version` and `Cargo.lock`; merging it builds and publishes that version, and pushes that leave the version alone build nothing. Per-PR bumps made every open PR conflict with every other on the version line.
  - Commit `Cargo.lock` whenever `Cargo.toml` changes.
- **PRs**:
  - **Short titles** (≤ 60 chars). One topic per PR — do not join unrelated changes with `+`, `&`, or `and`. Use the body for detail.
  - No changelog files (GitHub handles it).
  - **One PR per piece of work.** Incidental fixes found mid-task (a flaky test, a stale default, a typo) ride along in the current PR and are noted in its body; they are not a second topic and never a new PR.
  - **Check the queue before opening.** Run `gh pr list` first. If an open PR touches the same crates, or the new work needs it, build on that PR instead of opening beside it.
  - **Keep the ready queue short**: at most 3 ready-for-review PRs open at once. At the limit, merge, fix or close one before marking another ready. A ready PR that stops merging cleanly is fixed the same day or moved back to draft.
  - **Stack only dependent work that needs separate review**, such as a large change whose parts are reviewed on their own. Base each layer on the one below from the start; never retrofit independent PRs into a stack.
  - **Never rewrite another session's branch.** Bring a PR or stack layer up to date by merging `main` (or the layer below) into it, not by rebasing and force-pushing.
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
- ccache must be installed for development builds