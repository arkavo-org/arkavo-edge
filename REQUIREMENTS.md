# Arkavo Edge — Open‑Source Requirements

*Last updated: 26 May 2025*

> **Goal** Provide an OSS toolkit that matches or exceeds the developer‑centric capabilities of **Claude Code** and **Aider** while keeping proprietary logic private.
>
> **Audience** AI‑agent developers and framework maintainers who need a secure, cost‑smart runtime and CLI for multi‑file code transformations.

---

## Scope Overview

| Layer                 | Included in OSS                                                                            | Proprietary / Closed                             |
|-----------------------|--------------------------------------------------------------------------------------------|--------------------------------------------------|
| **CLI Core**          | `arkavo` binary, command parser, multi‑step agent loop                                     | –                                                |
| **Terminal UI**       | GPU‑accelerated terminal integration                                                       | –                                                |
| **Repo Mapper**       | Builds semantic map of repo; tracks changed files                                          | Advanced heuristics for file‑selection & ranking |
| **Git Integration**   | Auto‑commit, branch handling, unified‑diff previews                                        | Commit‑message LLM prompt templates              |
| **Protocol Adapters** | MCP & A2A client impls                                                                     | Smart Router decision engine                     |
| **Encryption**        | OpenTDF wrapping, local KMS support                                                        | Automated sensitivity tagging                    |
| **Edge Vault CE**     | Web UI, CRUD APIs, SQLite driver                                                           | Enterprise policy engine, RBAC, SSO              |
| **Test Harness**      | AI domain analysis, property discovery, state exploration, chaos injection, mobile bridges | Advanced bug pattern detection                   |

---

## Functional Requirements

### CLI Commands

| Command          | Description                                                        |
|------------------|--------------------------------------------------------------------|
| `arkavo chat`    | Conversational agent with repo context & streaming diff previews.  |
| `arkavo plan`    | Generates a change plan (tasks + affected files) before code edit. |
| `arkavo apply`   | Executes plan, writes files, commits with descriptive message.     |
| `arkavo test`    | AI‑driven intelligent test generation and exploration.             |
| `arkavo vault …` | Import/export notes to **Edge Vault**.                             |

### Repository Awareness

1. Parse up to **10 000 source files**; create a vector + symbol index (SQLite).
2. Support multi‑file refactors; ensure compilable compile‑unit after apply.
3. Produce **unified diffs** for every LLM edit ([aider.chat](https://aider.chat/docs/unified-diffs.html?utm_source=chatgpt.com)).

### Git Workflow

1. Initialize repo if absent.
2. Auto‑create feature branch `arkavo/<timestamp>`.
3. After each `apply`, commit with an AI‑generated message.
4. Provide `arkavo undo` to revert last commit.

### Protocol Layer

- Full MCP v1.0 and A2A 2025‑04 specs.
- Streaming chunk support with back‑pressure.

### Security & Privacy

- OpenTDF AES‑256 envelope on **all** outbound payloads.

### Edge Vault (Community Edition)

- Arkavo Community Web on `localhost:8191`.
- Supports Markdown docs and code snippets with tag search.

### Test Harness (Intelligent Test Generation)

#### Overview
The test harness provides AI‑driven intelligent test generation that understands domain models, discovers edge cases, and systematically finds behavioral bugs that humans miss.

#### Core Components

1. **AI Intelligence Engine**
  - Domain model analysis and comprehension
  - Property‑based test generation with invariant discovery
  - State space exploration with automatic minimization
  - Edge case generation beyond human imagination
  - Metamorphic relation identification

2. **Execution Infrastructure**
  - Native FFI bridges (Swift/Kotlin) for direct state manipulation
  - Hot‑reload test injection without rebuilds
  - Snapshot/restore with branching execution paths
  - Deterministic chaos injection
  - UI interaction for agents (implementation solutions like idb_companion, AppleScript communicated without requiring knowledge of their complexity)

3. **Test Generation Modes**
  - **Intelligent Mode**: AI autonomously explores and finds bugs
  - **Property Mode**: Discovers and verifies system invariants
  - **Chaos Mode**: Injects failures to test resilience
  - **Edge Case Mode**: Generates unusual but valid scenarios

4. **Platform Support**
  - **Languages**: Rust core with native bridges
  - **Mobile**: iOS (XCUITest), Android (Espresso)
  - **Backend**: Direct API and state testing
  - **Performance**: Sub‑50ms execution, no build cycles

5. **Secondary Features**
  - BDD/Gherkin parsing for business communication
  - Natural language test specifications
  - Business‑readable reports in multiple formats

#### Test Commands

| Command                    | Description                         |
|----------------------------|-------------------------------------|
| `arkavo test`              | Run intelligent test generation     |
| `arkavo test --explore`    | AI explores app states to find bugs |
| `arkavo test --properties` | Discover and verify invariants      |
| `arkavo test --chaos`      | Inject controlled failures          |
| `arkavo test --edge-cases` | Generate edge cases for modules     |
| `arkavo test --bdd`        | Execute business‑readable scenarios |

#### Output Formats
- Technical bug reports with minimal reproductions
- JUnit XML for CI integration
- Business‑readable HTML summaries
- Video recordings of failures (mobile)

---

## Non‑Functional Requirements

| Attribute         | Requirement                                                  |
|-------------------|--------------------------------------------------------------|
| **Performance**   | ≤50 ms from router response to diff render; 60 fps scroll.   |
| **Footprint**     | Binary ≤15 MB; RAM ≤150 MB typical.                          |
| **Portability**   | macOS (arm64), Linux (x64/aarch64).                          |
| **Build & CI**    | `cargo build --release`; GitHub Actions with Zig MUSL build. |
| **Quality Gates** | `cargo clippy -D warnings`, test coverage ≥85 %.             |
| **License**       | MIT (code), CC‑BY‑4.0 (docs).                                |

---

## Out of Scope (Open Source)

- Smart Router (cost/capability routing).
- Automated data‑tagging & sensitivity heuristics.
- Enterprise SSO, fine‑grained RBAC, analytics backend.
- Proprietary model weights.
- Cloud‑based device farms for testing.
- Advanced bug pattern detection algorithms.

---

## Acceptance Criteria

1. `arkavo plan` lists tasks and files for a refactor across ≥3 files.
2. `arkavo apply` commits unified diff and code compiles (language‑specific check).
3. `arkavo test --explore` discovers at least one behavioral bug in sample app.
4. `arkavo test --properties` generates valid invariants for domain model.
5. Mobile bridge successfully manipulates app state without UI.
6. Encryption on/off verified by round‑trip unit tests.
7. CI pipeline green on macOS & Linux runners.

---

## Contribution Guidelines (Abridged)

1. Fork → Branch → PR against `main`.
2. Sign DCO in each commit.
3. Include tests + docs for new code.
4. No proprietary internals in PRs.

---

## 🎯 Feature Summary & Priority

* **P0 — MVP**
  * Conversational CLI with repo context and unified‑diff edits
  * Git auto‑commit & `undo`
  * MCP & A2A client (stub router)
  * OpenTDF encryption (local Arkavo Keystore)
  * GPU terminal UI
  * Edge Vault basic (SQLite + web UI)

* **P1 — Near‑Term**
  * `plan` step with task graph
  * Intelligent test generation with property discovery
  * Mobile testing bridges (iOS/Android)
  * Multi‑language repo indexing
  * HashiCorp Vault key backend

* **P2 — Future**
  * Windows build
  * Postgres storage & dashboards
  * Fine‑tune on private knowledge base
  * Analytics & policy editor
  * Cloud device farm integration

---

## Recommendations & Next Steps

1. **Developer Experience First** Focus engineering sprints on the P0 feature set to deliver a fast, friction‑free first run and tight edit‑apply loop.
2. **Highlight Security Differentiators** Feature OpenTDF encryption and on‑prem/offline modes prominently in docs and marketing collateral.
3. **Emphasize AI Intelligence** Market the test harness as finding bugs developers don't know exist, not just running predefined tests.
4. **Document the Boundary** Provide architectural diagrams and API contracts that clearly separate OSS modules from proprietary Smart Router components.
5. **Cultivate Community** Launch a Discord/Matrix space, tag good first issues, and schedule monthly office hours to drive open‑core adoption.
6. **Execute Roadmap** Proceed with licensing, public roadmap publication, design‑partner pilots, and benchmark shoots‑out versus Claude Code & Aider.


---

## Challenges, Risks & Mitigations

| Area                                   | Risk / Ambiguity                                             | Mitigation                                                                                                              |
|----------------------------------------|--------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------|
| **Multi‑file Refactor**                | LLM edits may leave project uncompilable & language‑specific | **P0**: run formatter & linter post‑edit, auto‑rollback on failure  **P1**: plug in language servers for compile checks |
| **Repo Mapper vs. Ranking Heuristics** | OSS mapper may feel weak without proprietary ranking         | Ship baseline TF‑IDF + symbol graph; Smart Router adds advanced weighting                                               |
| **Aggressive Perf Targets**            | 15 MB binary & 50 ms render may be hard                      | Continuous perf tests in CI; enforce crate‑level size budgets                                                           |
| **Ghostty Dependency**                 | Windows timeline tied to Ghostty DLL                         | Provide TTY fallback; track upstream milestones                                                                         |
| **Router Boundary**                    | Need clear interface to closed Smart Router                  | Define `router.toml` + gRPC stub; ship pass‑through router with BYO key                                                 |
| **LLM Connectivity OOTB**              | Users must supply model creds                                | CLI setup wizard; optional Ollama local model download                                                                  |
| **Commit Messages**                    | OSS lacks AI commit summaries                                | Basic template (`feat: {task}`) in OSS; advanced prompts proprietary                                                    |
| **Edge Vault CE Value**                | Utility limited without RBAC                                 | Emphasise offline KB use cases; CSV import/export                                                                       |
| **Mobile Testing Complexity**          | FFI bridges add build complexity                             | Pre‑built binaries for common platforms; Docker images with bridges installed                                           |
| **AI Test Determinism**                | LLM‑generated tests may be flaky                             | Seed‑based generation; snapshot test outputs for regression detection                                                   |

---
