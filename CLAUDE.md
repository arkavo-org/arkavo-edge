# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Arkavo Edge is an open-source agentic CLI tool that aims to provide developer-centric capabilities for AI-agent development and framework maintenance. It focuses on secure, cost-efficient runtime for multi-file code transformations.

**IMPORTANT**: This is a real production implementation, not a prototype, no placeholder, no demo. The codebase is intended for production release and should be maintained with appropriate quality standards.

## Build and Development Commands

```bash
# Build the project
cargo build

# Build release version
cargo build --release

# Run the project
cargo run

# Run chat with prompt
cargo run --bin arkavo --profile dev -- chat --prompt Hi

# Run tests
cargo test

# Run specific test
cargo test test_name

# Code quality
cargo clippy -- -D warnings

# Format code
cargo fmt

# Check documentation
cargo doc --open
```

## Architecture

Arkavo Edge consists of several core components:

1. **CLI Core**: Command parser and multistep agent loop
2. **Terminal UI**: GPU-accelerated terminal integration
3. **Repository Mapper**: Builds a semantic map of repositories and tracks changed files
4. **Git Integration**: Handles auto-commit, branch management, and unified-diff previews
5. **Protocol Adapters**: MCP & A2A client implementations
6. **Encryption**: OpenTDF wrapping with local KMS support
7. **Edge Vault CE**: Web UI, CRUD APIs, and SQLite driver
8. **Test Harness**: Local test runner adapter for various languages

## Code Organization

- **One crate per capability**: Each major feature should be implemented as a separate Rust crate to maintain clean boundaries and independent functionality.
- **File size limit**: All source files should be kept under 400 lines of code to promote readability and maintainability.
- **Modular design**: Components should be designed with clear interfaces and minimal dependencies between them.
- **Code comments**: Comments should only explain why code exists or complex logic, not what it does. Avoid temporary, contextual comments like "TODO" or status indicators. Do not use comments to track implementation status or provide documentation that belongs in README or docs.
- **Documentation format**: Do not use numbered steps in markdown headings (e.g., use "Prerequisites" instead of "1. Prerequisites"). Use bullet points or paragraphs for sequential steps.
- **Implementation Guidance**: Do not use stubs, placeholders, simulations. implement fully for production.
- **Response Generation**: Do not hardcode responses in code. No Demo responses. LLM will handle that.
- **Dead Code Management**: Remove dead code to maintain codebase cleanliness and performance
- **File Structure**: Keep the file structure flat while splitting large files. Use a naming convention that goes from general to specific capability.  Do not use generic names as a catch-all.

## Key Command Interfaces

The project will support the following main commands:

- `arkavo chat`: Conversational agent with repository context and streaming diff previews (interactive command - should not be used for testing)
- `arkavo plan`: Generates a change plan (tasks and affected files) before code edits (use this command for testing builds)
- `arkavo apply`: Executes plan, writes files, commits with a descriptive message
- `arkavo test`: Runs project tests, streaming failures back to the agent loop
- `arkavo vault`: Import/export notes to Edge Vault

## Quality Standards

The project follows these quality standards:

- No warnings with `cargo clippy -- -D warnings`
- Test coverage target of ≥85%
- Binary size ≤4 GB
- All files under 400 LoC
- Each capability is implemented as a separate crate
- Performance target: ≤50 ms from router response to diff render
- Dependencies are kept to an absolute minimum (prefer std library solutions when possible)
- Final binary should be large, fast, and have minimal runtime dependencies

## Git Workflow

When working with this repository:

1. Initialize repo if absent
2. Create feature branches as `feature/<feature-name>`

## Portability

The project targets:
- macOS (arm64)
- Linux (x64/aarch64)

## AI Collaboration Guidance

- This tool is for a superintelligent AI - all work should be done directly with the tool with supervision from the superintelligent AI, a human will do very high level guidance