# Arkavo Edge — Open‑Source Requirements

*Last updated: 26 May 2025*

> **Goal** Provide an OSS toolkit that matches or exceeds the developer‑centric capabilities of **Claude Code** and **Aider** while keeping proprietary logic private.
>
> **Audience** AI‑agent developers and framework maintainers who need a secure, cost‑smart runtime and CLI for multi‑file code transformations.

---

## Scope Overview

| Layer                 | Included in OSS                                        | Proprietary / Closed                             |
|-----------------------|--------------------------------------------------------|--------------------------------------------------|
| **CLI Core**          | `arkavo` binary, command parser, multi‑step agent loop | –                                                |
| **Terminal UI**       | GPU‑accelerated terminal integration                   | –                                                |
| **Repo Mapper**       | Builds semantic map of repo; tracks changed files      | Advanced heuristics for file‑selection & ranking |
| **Git Integration**   | Auto‑commit, branch handling, unified‑diff previews    | Commit‑message LLM prompt templates              |
| **Protocol Adapters** | MCP & A2A client impls                                 | Smart Router decision engine                     |
| **Encryption**        | OpenTDF wrapping, local KMS support                    | Automated sensitivity tagging                    |
| **Edge Vault CE**     | Web UI, CRUD APIs, SQLite driver                       | Enterprise policy engine, RBAC, SSO              |
| **Test Harness**      | BDD/Gherkin parser, AI test mapper, iOS/Android bridge, dynamic test injection | Flaky‑test heuristics, cloud runners             |

---

## Functional Requirements

### CLI Commands

| Command          | Description                                                        |
|------------------|--------------------------------------------------------------------|
| `arkavo chat`    | Conversational agent with repo context & streaming diff previews.  |
| `arkavo plan`    | Generates a change plan (tasks + affected files) before code edit. |
| `arkavo apply`   | Executes plan, writes files, commits with descriptive message.     |
| `arkavo test`    | AI‑driven test execution with BDD support & mobile app testing.    |
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

### Test Harness (AI‑Powered Testing)

#### Overview
The test harness provides a unified AI‑driven testing framework that bridges business‑readable BDD specifications with deep behavioral testing capabilities for web and mobile applications.

#### Core Components

1. **Gherkin/BDD Layer**
  - Parse `.feature` files with Given/When/Then syntax
  - Natural language test specification support
  - AI‑powered step mapping (no manual bindings required)
  - Business‑readable test reports in Markdown/HTML

2. **AI Planning Engine**
  - LLM‑based test generation from natural language objectives
  - Property‑based testing with invariant discovery
  - Stateful exploration with automatic minimization
  - Metamorphic relation identification

3. **Mobile Bridge (iOS/Android)**
  - Native FFI bridge to Swift/Kotlin test frameworks
  - Direct app state inspection and mutation
  - Snapshot/restore with branching execution paths
  - Hot‑reload test injection without rebuilds

4. **Execution Modes**
  - **BDD Mode**: Execute Gherkin scenarios with AI step interpretation
  - **Exploratory Mode**: AI autonomously explores app states
  - **Property Mode**: Continuous invariant checking
  - **Chaos Mode**: Deterministic failure injection

5. **Performance Features**
  - Dynamic test compilation (no xcodebuild cycles)
  - Parallel execution across device pool
  - Sub‑50ms action execution via native bridges
  - Memory‑efficient state snapshots with LMDB

#### Supported Platforms
- **Languages**: Rust core with Swift/Kotlin bridges
- **Mobile**: iOS (XCUITest), Android (Espresso/UiAutomator)
- **Web**: Selenium/Playwright adapters
- **Desktop**: Native app testing via accessibility APIs

#### Test Commands

| Command                        | Description                                                    |
|--------------------------------|----------------------------------------------------------------|
| `arkavo test`                  | Run all tests in current project                               |
| `arkavo test --bdd`            | Execute BDD scenarios from `.feature` files                    |
| `arkavo test --explore`        | AI explores app for specified duration                         |
| `arkavo test --chaos`          | Inject failures while testing                                  |
| `arkavo test --device iPhone`  | Target specific device/simulator                               |
| `arkavo test --plan <goal>`    | Generate test plan from natural language goal                  |

#### Output Formats
- JUnit XML for CI integration
- Business‑readable HTML reports
- Slack/Discord webhooks
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
- Advanced flaky test detection algorithms.

---

## Acceptance Criteria

1. `arkavo plan` lists tasks and files for a refactor across ≥3 files.
2. `arkavo apply` commits unified diff and code compiles (language‑specific check).
3. `arkavo test` executes BDD scenarios and generates business reports.
4. `arkavo test --explore` discovers at least one invariant violation in sample app.
5. Mobile bridge successfully snapshots and restores app state.
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
  * AI‑powered test harness with BDD support
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
3. **Expand Testing Strategy** Maintain ≥85 % coverage *and* add performance regression tests (latency & memory) in CI.
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
