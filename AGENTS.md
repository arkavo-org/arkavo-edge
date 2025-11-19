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
  - **Size**: Max 400 lines per file. Split larger files immediately.
  - **Style**: `cargo fmt` required. No dead code (`#[allow(dead_code)]` forbidden).
  - **Comments**: Explain *why*, not *what*. No TODOs or status tracking.
  - **Safety**: No hardcoded responses. LLM handles generation.
- **Documentation**:
  - No numbered headings in Markdown.
- **Performance**:
  - Router response ≤ 50ms.
  - Binary ≤ 60MB.
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

## Architecture & Tech Stack
- **Cross-Platform**: macOS (arm64), Linux (x64/aarch64), Windows (x86_64).
- **Security**:
  - **NO OpenSSL**: Use `rustls` exclusively (musl compatibility).
  - **Secrets**: Never commit or write to docs any API keys.
- **Components**:
  - **CLI/TUI**: `crossterm`, `ratatui`.
  - **Data**: SQLite (`crates/arkavo-vault`).
  - **Networking**: mDNS, MCP adapters.
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
  - Bump semver in `Cargo.toml` on feature completion.
  - Commit `Cargo.lock` whenever `Cargo.toml` changes.
- **PRs**: Short titles. No changelog files (GitHub handles it).
- **Docs**: Technical docs in `docs/`. 

## Agent Configuration
- **Autonomous**: Auto-detect capabilities.  No manual configuration.
- **Orchestration**: Optional centralized control plane.

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
cargo run -p arkavo -- chat --prompt "Hello"
```

## 7. Environment Variables
- `ARKAVO_DEBUG=1`: General debug logging.
- `ARKAVO_DEBUG_CHAT=1`: Chat/Template/Token debug.
