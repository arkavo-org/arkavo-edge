# AGENTS.md

This file provides guidance to AI assistants (Claude Code, Gemini, etc.) when working with code in this repository.

## Project Overview

Arkavo Edge is an open-source agentic CLI tool that aims to provide developer-centric capabilities for AI-agent development and framework maintenance. It focuses on secure, cost-efficient runtime for multi-file code transformations.

**IMPORTANT**: This is a real production implementation, not a prototype, no placeholder, no demo. The codebase is intended for production release and should be maintained with appropriate quality standards.

## Project Structure and Organization

- **One crate per capability**: Each major feature should be implemented as a separate Rust crate to maintain clean boundaries and independent functionality.
- **File size limit**: All source files should be kept under 400 lines of code to promote readability and maintainability.
- **Modular design**: Components should be designed with clear interfaces and minimal dependencies between them.
- **Code comments**: Comments should only explain why code exists or complex logic, not what it does. Avoid temporary, contextual comments like "TODO" or status indicators. Do not use comments to track implementation status or provide documentation that belongs in README or docs.
- **Documentation format**: Do not use numbered steps in markdown headings (e.g., use "Prerequisites" instead of "1. Prerequisites"). Use bullet points or paragraphs for sequential steps.
- **Implementation Guidance**: Do not use stubs, placeholders, simulations. implement fully for production.
- **Response Generation**: Do not hardcode responses in code. No Demo responses. LLM will handle that.
- **Dead Code Management**: Remove dead code to maintain codebase cleanliness and performance
- **File Structure**: Keep the file structure flat while splitting large files. Use a naming convention that goes from general to specific capability. Do not use generic names as a catch-all.

- **Documentation files**: Technical documentation, implementation guides, and historical documents should be placed in the `docs/` directory. The following files should remain in root:
  - `README.md` - Main project documentation
  - `CLAUDE.md` - AI assistant instructions
  - `THIRD-PARTY-LICENSES.md` - License information
  - Crate-specific `README.md` files remain in their respective crate directories
- **Test files**:
  - Integration tests should be placed in the `tests/` directory at the crate level
  - Unit tests should remain as inline `#[cfg(test)]` modules in source files (standard Rust convention)
  - Temporary test scripts or debugging utilities should be removed rather than kept in the repository

## Build, Test, and Development Commands

```bash
# Build the project
cargo build

# Build release version
cargo build --release

# Run the project
cargo run

# Run chat with prompt (default, no embeddings)
cargo run -p arkavo -- chat --prompt Hi

# Run tests (standard)
cargo test

# Run tests with nextest (faster, recommended for development)
# Install once: cargo install cargo-nextest --locked
cargo nextest run

# Run specific test
cargo test test_name
# or with nextest:
cargo nextest run -E 'test(test_name)'

# Rerun only failed tests
cargo nextest run --failed

# Code quality
cargo clippy -- -D warnings

# Format code
cargo fmt

# Check documentation
cargo doc --open
```

## Coding Conventions and Style

- **One crate per capability**: Each major feature should be implemented as a separate Rust crate to maintain clean boundaries and independent functionality.
- **File size limit**: All source files should be kept under 400 lines of code to promote readability and maintainability.
- **Modular design**: Components should be designed with clear interfaces and minimal dependencies between them.
- **Code comments**: Comments should only explain why code exists or complex logic, not what it does. Avoid temporary, contextual comments like "TODO" or status indicators. Do not use comments to track implementation status or provide documentation that belongs in README or docs.
- **Documentation format**: Do not use numbered steps in markdown headings (e.g., use "Prerequisites" instead of "1. Prerequisites"). Use bullet points or paragraphs for sequential steps.
- **Implementation Guidance**: Do not use stubs, placeholders, simulations. implement fully for production.
- **Response Generation**: Do not hardcode responses in code. No Demo responses. LLM will handle that.
- **Dead Code Management**: Remove dead code to maintain codebase cleanliness and performance
- **File Structure**: Keep the file structure flat while splitting large files. Use a naming convention that goes from general to specific capability. Do not use generic names as a catch-all.

## Architecture and Design Patterns

Arkavo Edge consists of several core components:

1.  **CLI Core**: Command parser and multistep agent loop
2.  **Terminal UI**: GPU-accelerated terminal integration
3.  **Repository Mapper**: Builds a semantic map of repositories and tracks changed files
4.  **Git Integration**: Handles auto-commit, branch management, and unified-diff previews
5.  **Protocol Adapters**: MCP & A2A client implementations
6.  **Encryption**: OpenTDF wrapping with local KMS support
7.  **Edge Vault CE**: Web UI, CRUD APIs, and SQLite driver
8.  **Test Harness**: Local test runner adapter for various languages

## Testing Guidelines

- **Bug fixes MUST have regression tests** - Every bug fix must include a test that would have caught the bug. Add tests to `.github/workflows/regression.yaml` or create issue-specific test files in `tests/` directories.
- Integration tests should be placed in the `tests/` directory at the crate level.
- Unit tests should remain as inline `#[cfg(test)]` modules in source files (standard Rust convention).
- Temporary test scripts or debugging utilities should be removed rather than kept in the repository.

## Security Considerations

- **No OpenSSL dependency** - Use rustls for TLS to ensure cross-compilation compatibility (especially for musl targets).

## Pull Request Guidelines

- Keep PR titles short and not "feat:". the reason is this is shown prominently in Github next to files and folders.
- Do not keep a change log file; github handles that functionality.
- Each feature branch needs to bump the appropriate semver version. No release branches.
- When version is updated, cargo build, then also git commit the Cargo.lock with the Cargo.toml.

## Key Command Interfaces

The project will support the following main commands:

- `arkavo ui`: User-interface for agent orchestration
- `arkavo agent run`: Start an agent
- `arkavo chat`: Simple conversational chat interface without TUI (interactive command - should not be used for testing)
- `arkavo terminal`: Terminal UI with streaming LLM responses and full visual interface


## Quality Standards

The project follows these quality standards:

- No warnings with `cargo clippy -- -D warnings`
- Test coverage target of ≥85%
- Binary size ≤60 MB
- All files under 400 LoC
- Each capability is implemented as a separate crate
- Performance target: ≤50 ms from router response to diff render
- Dependencies are kept to an absolute minimum (prefer std library solutions when possible)
- Final binary should be large, fast, and have minimal runtime dependencies
- **No OpenSSL dependency** - Use rustls for TLS to ensure cross-compilation compatibility (especially for musl targets)

## Git Workflow

When working with this repository:

1.  Initialize repo if absent
2.  Create feature branches as `feature/<feature-name>`

## Portability

The project targets:

- macOS (arm64)
- Linux (x64/aarch64)
- Windows (x86_64) - Limited support without iOS testing capabilities

All implementations must work across:

- ✅ On simulators
- ✅ On real devices (with proper signing)
- ✅ Across different user home directories
- ✅ On different platforms (iOS, tvOS, etc.)
- ✅ In CI/CD environments

Avoid hardcoded paths, platform-specific assumptions, or environment-dependent configurations. Use relative paths, dynamic discovery, and embedded resources where possible.

## AI-Driven Configuration

The configuration of Arkavo Edge is a dynamic process handled by the AI agent at runtime, rather than through static files, command-line parameters, or environment variables set by a human user. The agent configures its operations through a combination of inquiry and observation:

- **Interactive Dialogue with the User:** When the agent requires input or needs to make a decision with multiple valid options, it will directly ask the human supervisor for guidance. For example, instead of requiring a pre-set configuration for a new feature branch, the agent would ask, "What should I name the feature branch for this task?" This conversational approach makes configuration contextual and task-specific.

- **Environmental and Capability Awareness:** The agent autonomously assesses its environment to gather necessary configuration details. It can detect the operating system, understand the current state of the Git repository, and map the codebase. Furthermore, the agent is aware of its own compiled capabilities (e.g., whether it was built with `embeddings` support) and will adjust its available actions and strategies accordingly, without needing to be explicitly told which version is running.

## Development Principles

- Keep required third-party tools to a minimum. Zero configuration is required for humans.

## Chat Session Management for Small Models

The chat command includes sophisticated session management to handle small language models (≤1B parameters) more effectively:

### Smart Repository Context Injection
Prevents small models from getting confused by documentation:

#### Policy
- **≤1B models**: No repo context by default (just system prompt + conversation)
- **≥2B models**: Auto-inject context based on query relevance
- **Override**: Use `--repo-context {auto|on|off}` or `/context` command

#### Auto-Detection
Context is only injected for code/doc related queries:
- Math expressions like "2+2=" → No context
- Greetings like "hello" → No context  
- Keywords like "build", "cargo", "error:" → Context injected
- File paths or code terms → Context injected

#### Compression
When context is injected, it's compressed to:
- 270M models: 200 tokens max
- 1B models: 300 tokens max
- 2B models: 500 tokens max
- Larger models: 1000 tokens max

### History Sanitization
- Automatically balances unclosed code fences in conversation history
- Removes trailing role artifacts ("Assistant:", "User:", etc.)
- Adds buffer after code introduction patterns to prevent immediate fence generation

### Session Compatibility Checking
- Sessions store metadata: chat template hash, system prompt hash, model size
- Automatically starts new session if:
  - Model changes (even quantization differences)
  - Chat template changes
  - System prompt changes
- Clear message: "Session skipped: [reason] (starting fresh)"

### Smart Defaults by Model Size
- **270M models**: 2 turn history limit
- **1-2B models**: 4 turn history limit  
- **Larger models**: 10 turn history (default)
- Override with `ARKAVO_MAX_HISTORY_TURNS` environment variable

### Interactive Commands
- `/new` - Start fresh session (replaces --new-session flag)
- `/clear` - Clear current session history
- `/context {auto|on|off}` - Control repository context injection
- `/history` - Show session statistics (turns, tokens, fence parity)
- `/history --last N` - Display last N messages
- `/switch <id>` - Switch between sessions

### Debug Features
When `ARKAVO_DEBUG_CHAT=1`:
- Shows template/prompt preview (first/last 200 chars)
- Reports token and message counts
- Checks code fence parity before generation
- Warns about unbalanced fences that may cause issues
- Shows context injection decisions ("RepoCtx: injected=N tokens" or "RepoCtx: skipped (reason)")

## Memories

- Faking success is worse than an honest failure
- do not use conventional commits
- **Bug fixes MUST have regression tests** - Every bug fix must include a test that would have caught the bug. Add tests to `.github/workflows/regression.yaml` or create issue-specific test files in `tests/` directories
- **IMPORTANT: simctl does NOT support tap/touch/swipe commands**. The following commands are INVALID and do not exist:
  - `xcrun simctl io <device> tap <x> <y>` - DOES NOT EXIST
  - `xcrun simctl io <device> touch <x>,<y>` - DOES NOT EXIST
  - `xcrun simctl io <device> swipe` - DOES NOT EXIST
  - `xcrun simctl io <device> sendkey` - DOES NOT EXIST
  - Valid simctl io commands are ONLY: enumerate, poll, recordVideo, screenshot
  - For UI automation use: IDB, XCTest, or AppleScript - NOT simctl
- run clippy and cargo fmt before each git push
- ProTip! Add .patch or .diff to the end of URLs for Git's plaintext views.
- keep PR titles short and not "feat:". the reason is this is shown prominently in Github next to files and folders
- each feature branch needs to bump the appropriate semver version. No release branches
- Do not keep a change log file; github handles that functionality
- minimize key mappings.  Ideally the app works intuitively.  a power user can use Natural Language to set a key mapping.
- when a context is nearing completion, write to docs/longterm-memory.md about any important details or reveleations that are currently not there.  and on each new context read it
- when version is updated, cargo build, then also git commit the Cargo.lock with the Cargo.toml

## Event Storage Configuration

- **Event Retention**: 24 hours by default (configurable via `ARKAVO_EVENT_RETENTION_HOURS` env var)
- **Maximum Events**: 100,000 events per session (configurable via `ARKAVO_MAX_EVENTS_PER_SESSION`)
- **Storage Limits**: Automatic cleanup when database exceeds 1GB
- **Pruning Strategy**: Keep most recent events when limits are reached

## Environment Variables

- **ARKAVO_NO_TERMINAL_RELAUNCH**: Disables automatic terminal relaunch on macOS (for testing/automation)
- **ARKAVO_EVENT_RETENTION_HOURS**: Sets event retention period (default: 24)
- **ARKAVO_MAX_EVENTS_PER_SESSION**: Maximum events per session (default: 100000)
- **ARKAVO_DEBUG**: Enables debug logging
- **ARKAVO_DEBUG_CHAT**: Enables verbose debug output for chat/terminal commands including:
  - Template and prompt preview (first/last 200 chars)
  - Token counts and message counts
  - Code fence parity checking
  - Session compatibility checks
  - Repository context injection decisions
- **ARKAVO_MASTER_KEY**: Sets master encryption key
- **ARKAVO_MAX_TOKENS**: Sets default max tokens for LLM generation (default: 4096)
- **ARKAVO_MAX_HISTORY_TURNS**: Maximum conversation turns to include in context (default: 2 for <1B models, 10 for larger)
- **ARKAVO_REPO_CONTEXT**: Controls repository context injection: auto|on|off (default: auto)
- remove dead code instead of adding `#[allow(dead_code)]`
- Debug output is now controlled by the ARKAVO_DEBUG_CHAT environment variable. By default, all those
  verbose debug statements are suppressed. To enable them for debugging, users can run:

  ARKAVO_DEBUG_CHAT=1 arkavo chat --prompt "test"
- don't build --release while we are in development mode, use debug
- **Windows builds exclude C++ dependencies**: The `windows-default` feature explicitly excludes llama-cpp and other C++ dependencies (like candle) to avoid MSVC linker issues. Windows builds only include pure Rust features: memory, mdns, and llm-remote. If you add new dependencies, ensure they don't hardcode C++ features that would be pulled into Windows builds.