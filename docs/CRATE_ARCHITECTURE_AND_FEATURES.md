# Arkavo Edge Crate Architecture and Feature Summary

This document provides a comprehensive overview of the Arkavo Edge codebase, grouping its 55+ crates into logical categories and summarizing their key features. It also outlines potential future refactoring opportunities to improve maintainability and performance.

## 1. Core & CLI

These crates form the backbone of the application, handling entry points, command parsing, and the main execution loop.

| Crate | Description & Key Features |
| :--- | :--- |
| **`arkavo`** | **Main Binary & Entry Point**<br>- Agentic CLI for complex tasks.<br>- Multi-model orchestration (Gemini, Claude, Local).<br>- Context-aware interactions with long-term memory.<br>- Cross-platform (macOS, Linux, Windows). |
| **`arkavo-cli`** | **CLI Implementation & Execution Loop**<br>- Progressive tool disclosure to save tokens.<br>- Iterative agent loop (plan → execute → refine).<br>- Router-integrated quality gates.<br>- Interactive chat and task modes.<br>- Human-readable feedback and status reporting. |
| **`arkavo-workspace`** | **Workspace Management**<br>- Standardized project management tools.<br>- Recursive file discovery and listing.<br>- Subprocess management and execution.<br>- Environment isolation for autonomous tasks.<br>- Workspace health monitoring. |
| **`arkavo-config-bundle`** | **Configuration Management**<br>- Standardized config bundle format.<br>- Role/attribute-based targeting.<br>- Entitlement and secret management.<br>- Automated rotation policies. |

## 2. LLM & AI Orchestration

These crates manage the intelligence layer, including model inference, routing, context, and specialized agent capabilities.

| Crate | Description & Key Features |
| :--- | :--- |
| **`arkavo-llm`** | **Unified LLM Abstraction**<br>- Multi-provider orchestration (Gemini, Claude, Kimi, etc.).<br>- Unified message/role definitions.<br>- Real-time delta streaming.<br>- Integrated tool execution and validation.<br>- Multimodal support (text + images). |
| **`arkavo-router`** | **Intelligent Model Routing**<br>- Sub-100ms task classification.<br>- Dynamic routing between local (Gemma) and cloud (Gemini/Claude) models.<br>- Quality gate with auto-escalation.<br>- Budget-aware orchestration.<br>- Architect mode for complex task decomposition. |
| **`arkavo-context`** | **Context Management**<br>- Semantic context compression.<br>- Intelligent text chunking.<br>- Context deduplication.<br>- Dynamic prompt enrichment (files, problem statements).<br>- Compression metrics tracking. |
| **`arkavo-memory`** | **Long-Term Memory**<br>- Local-first vector search (HNSW).<br>- Embedded embedding models (no runtime download).<br>- SQLite persistence.<br>- Semantic categorization and retrieval.<br>- Privacy-focused (offline). |
| **`arkavo-orchestrator`** | **GitHub Task Orchestration**<br>- Webhook reception and routing.<br>- Intelligent issue analysis and complexity assessment.<br>- Agent assignment based on capability.<br>- Secure GitHub App authentication.<br>- Cognitive engine for task planning. |
| **`arkavo-hrm`** | **Hierarchical Reasoning**<br>- HRM-style task decomposition.<br>- Bounded execution bursts.<br>- Persistent task state storage.<br>- Context handover strategies.<br>- Loop detection and prevention. |
| **`arkavo-critic`** | **Response Verification**<br>- Pluggable verification pipeline.<br>- Schema and policy validation.<br>- Semantic coherence checks.<br>- Priority-ordered execution.<br>- Structured evidence reporting. |
| **`arkavo-ensemble`** | **Policy Ensemble**<br>- Multi-policy management and evaluation.<br>- Counterfactual evaluation on production inputs.<br>- Cumulative regret tracking.<br>- Automated promotion workflows.<br>- Statistical significance testing. |
| **`arkavo-autolearn`** | **Self-Healing & Learning**<br>- Automated anomaly detection and patching loop.<br>- LLM-based patch synthesis.<br>- Immune system verification (SAT/Invariant).<br>- Gossip-based patch propagation. |

## 3. Model Providers & Inference

Specialized crates for interfacing with specific AI models and inference engines.

| Crate | Description & Key Features |
| :--- | :--- |
| **`arkavo-gemini`** | **Google Gemini Integration**<br>- Streaming REST client (sub-second TTFT).<br>- Advanced function calling support.<br>- Live API (WebSocket) for multimodal.<br>- Concurrent tool dispatching. |
| **`arkavo-claude-code`** | **Anthropic Claude Integration**<br>- Claude Agent SDK support.<br>- Secure Node.js bridge.<br>- Workspace sandboxing.<br>- Policy-controlled execution. |
| **`arkavo-qwen`** | **Qwen Integration**<br>- Qwen-3 and Qwen-VL support.<br>- Multimodal understanding.<br>- Regional endpoint support (Intl/CN).<br>- Vision optimization helpers. |
| **`arkavo-deepseek`** | **DeepSeek Integration**<br>- Dual API support (OpenAI/Anthropic).<br>- DeepSeek-Reasoning model support.<br>- 128+ concurrent tool support.<br>- Strict JSON schema validation. |
| **`arkavo-kimi`** | **Moonshot Kimi Integration**<br>- Native Kimi API client.<br>- Function/tool calling support.<br>- Long-context window handling. |
| **`arkavo-llama-cpp`** | **Local Inference (High-Level)**<br>- High-level wrapper for llama.cpp.<br>- Auto-GPU acceleration (Metal/CUDA/Vulkan).<br>- Multi-format chat templates.<br>- Constrained decoding support. |
| **`arkavo-llama-cpp-sys`** | **Local Inference (Low-Level)**<br>- FFI bindings for llama.cpp.<br>- Automated build system (CMake).<br>- Static linking support. |
| **`arkavo-snpe`** | **Qualcomm SNPE Backend**<br>- Hardware acceleration for Snapdragon (DSP/NPU).<br>- Dynamic library loading (dlopen).<br>- UNO Q platform optimization.<br>- CPU fallback support. |

## 4. MCP (Model Context Protocol) & Tools

Crates implementing the Model Context Protocol and providing tools for agents.

| Crate | Description & Key Features |
| :--- | :--- |
| **`arkavo-mcp`** | **MCP Core Traits**<br>- Standard `Tool` trait definition.<br>- Schema-driven discovery.<br>- Async execution model.<br>- Protocol interoperability types. |
| **`arkavo-mcp-core`** | **MCP Types**<br>- Zero-dependency core types.<br>- JSON-RPC 2.0 compliance models.<br>- Unified tool schema definitions.<br>- Standardized error handling. |
| **`arkavo-mcp-runtime`** | **MCP Server Runtime**<br>- Dynamic client management.<br>- JSON-RPC server implementation.<br>- Built-in toolsets.<br>- Runtime persistence and discovery. |
| **`arkavo-mcp-tools`** | **Standard Tool Registry**<br>- **Security**: Semgrep, OSV, Syft.<br>- **GitHub**: PR reviews, checks, issues.<br>- **Testing**: Multi-language runners.<br>- **Git/FS**: Repo and file operations.<br>- **Context**: Web search, time, OS info. |
| **`arkavo-mcp-macos`** | **macOS Specific Tools**<br>- iOS Simulator management.<br>- XCUITest automation bridge.<br>- Diagnostic reporting.<br>- Non-interactive UI automation. |
| **`arkavo-mesh-tools`** | **Mesh Network Tools**<br>- mDNS agent discovery.<br>- Capability-based agent querying.<br>- Secure task delegation.<br>- Load-aware routing tools. |
| **`arkavo-code-search`** | **Code Analysis Tools**<br>- Ripgrep integration for fast search.<br>- Structural search/replace (Comby).<br>- Tree-sitter AST parsing.<br>- Multi-language support. |
| **`arkavo-browser`** | **Browser Automation**<br>- Chromium automation via WebDriver.<br>- Live DOM injection.<br>- Screenshot and visual state capture.<br>- Async interaction model. |
| **`arkavo-git`** | **Git Operations**<br>- Autonomous repo management.<br>- AI-generated commit messages.<br>- Unified diff generation.<br>- Safety checks and rollbacks. |
| **`arkavo-github`** | **GitHub Integration**<br>- Issue orchestration and discovery.<br>- Organization polling.<br>- Octocrab integration.<br>- Persistent state tracking. |
| **`arkavo-repo`** | **Repository Analysis**<br>- Semantic repository mapping.<br>- Change tracking and state monitoring.<br>- Intelligent ignore handling.<br>- Context optimization. |

## 5. UI & Frontend Generation

Crates focused on generating and rendering user interfaces.

| Crate | Description & Key Features |
| :--- | :--- |
| **`arkavo-ui-core`** | **UI Abstractions**<br>- Unified UI traits and types.<br>- LLM integration for UI events.<br>- Dual engine support (CEF/Web).<br>- Safe HTML rendering. |
| **`arkavo-ui-generator`** | **AI UI Generation**<br>- Text-to-UI generation pipeline.<br>- Vision-powered verification (screenshots).<br>- Automated planning and decomposition.<br>- Multimodal image pipeline. |
| **`arkavo-agui`** | **Agentic GUI Protocol**<br>- Real-time UI streaming.<br>- WebSocket protocol for live updates.<br>- System status monitoring.<br>- MCP tool integration for UI. |
| **`arkavo-cef`** | **CEF Integration**<br>- Native Chromium Embedded Framework.<br>- Zero-JS Rust-to-DOM manipulation.<br>- Sub-millisecond rendering.<br>- Async Unix domain socket transport. |
| **`arkavo-terminal`** | **Terminal UI**<br>- GPU-accelerated TUI (Ratatui).<br>- Multi-terminal management.<br>- Tree-sitter syntax highlighting.<br>- Vim/Helix editor integration. |

## 6. Protocols, Network & Security

Infrastructure for secure communication and data transport.

| Crate | Description & Key Features |
| :--- | :--- |
| **`arkavo-protocol`** | **Unified Protocols**<br>- mTLS security support.<br>- Dual MCP and A2A (Agent-to-Agent) support.<br>- HTTP/WebSocket transports.<br>- OpenRPC schema generation. |
| **`arkavo-config-transport`** | **Config Distribution**<br>- Secure A2A config transport.<br>- Signed envelopes.<br>- Orchestrator signature verification.<br>- Automated update handling. |
| **`arkavo-config-encryption`** | **Config Security**<br>- OpenTDF encryption.<br>- Attribute-Based Access Control (ABAC).<br>- KAS integration.<br>- Agent identity verification. |
| **`arkavo-crypto`** | **Cryptography**<br>- Ed25519 keypair management.<br>- Message signing and verification.<br>- Portable key formats.<br>- Identity standardization. |
| **`arkavo-device-identity`** | **Device Identity**<br>- Persistent hardware-bound IDs.<br>- Secure platform storage (Keychain/TPM).<br>- NPE (Non-Person Entity) support.<br>- Attestation integration. |
| **`arkavo-attestation`** | **Platform Attestation**<br>- Hardware-backed evidence (TPM/Secure Enclave).<br>- Security state detection.<br>- Honest reporting mechanism.<br>- Cross-platform support. |
| **`arkavo-authorization`** | **Access Control**<br>- OpenTDF Auth v2 integration.<br>- Entity resolution via ERS.<br>- Fine-grained ABAC.<br>- Fail-closed security model. |
| **`arkavo-tdf`** | **Trusted Data Format**<br>- Streaming TDF abstraction.<br>- ZTDF-JSON support.<br>- KAS client integration.<br>- Decoupled encryption/transport. |
| **`arkavo-tdf-iroh`** | **P2P Transport**<br>- Iroh-based blob transport.<br>- Content-addressed storage.<br>- Embedded P2P node.<br>- Optimized peer fetching. |
| **`arkavo-gossip`** | **Gossip Protocol**<br>- Epidemic message propagation.<br>- Quorum consensus (2/3).<br>- Zero-trust message verification.<br>- Anti-entropy synchronization. |

## 7. Infrastructure & Observability

Foundational crates for system health, monitoring, and correctness.

| Crate | Description & Key Features |
| :--- | :--- |
| **`arkavo-observability`** | **System Observability**<br>- Structured logging and tracing.<br>- Session-level correlation.<br>- OTLP auto-configuration.<br>- Health reporting. |
| **`arkavo-events`** | **Event Bus**<br>- Unified system-wide event model.<br>- Structured typed payloads.<br>- High-performance async writer.<br>- Audit-ready metadata. |
| **`arkavo-budget`** | **Cost Management**<br>- Real-time token/cost tracking.<br>- Configurable budget limits.<br>- Cost-aware model selection.<br>- Architect-level savings reports. |
| **`arkavo-debugger`** | **Debugging Tools**<br>- Session replay and forensics.<br>- Diagnostics API.<br>- Real-time health checks.<br>- Error pattern analysis. |
| **`arkavo-bench`** | **Benchmarking**<br>- SWE-bench integration.<br>- Parallel execution runner.<br>- Arkavo-assisted solver mode.<br>- Comparative analysis tools. |
| **`arkavo-dataflow`** | **Data Pipeline**<br>- Natural language pipeline creation.<br>- Structured DSL.<br>- Secure execution sandbox.<br>- Real-time monitoring. |
| **`arkavo-sbe`** | **Symbolic Boundary Evolution**<br>- Hierarchical policy layers (Invariant/Policy/Adaptive).<br>- Formal verification of invariants.<br>- Adaptive evolution with rollback.<br>- Persistent policy store. |
| **`arkavo-sat`** | **Formal Verification**<br>- SAT solver integration.<br>- CNF extraction from policies.<br>- Boundary probing.<br>- Policy stress testing. |
| **`arkavo-torg`** | **Constrained Decoding**<br>- TØR-G logic masking for LLMs.<br>- Formal language bridge.<br>- High-speed mask generation.<br>- Graph extraction from tokens. |
| **`arkavo-titan`** | **Runtime Monitoring**<br>- Low-latency (<5µs) anomaly detection.<br>- Zero-copy inspection.<br>- Boundary violation detection.<br>- Statistical drift monitoring. |
| **`arkavo-registration`** | **Agent Registration**<br>- Cryptographic onboarding.<br>- QR/URL-based discovery.<br>- mDNS network registration.<br>- Portable identity descriptors. |

## Future Refactoring Contemplations

The current architecture is highly modular, which is excellent for separation of concerns but introduces complexity in dependency management and build times.

### 1. Consolidate MCP Crates
**Proposal**: Merge `arkavo-mcp`, `arkavo-mcp-core`, and potentially `arkavo-mcp-runtime` into a single workspace or crate.
- **Why**: The distinction between "core traits", "core types", and "runtime" causes friction when updating protocol definitions. A single `arkavo-mcp` crate with feature flags (e.g., `features = ["server", "client", "tools"]`) would simplify usage.

### 2. Unify LLM Provider Logic
**Proposal**: The `arkavo-llm` crate already acts as an abstraction, but we have separate heavy crates for `arkavo-gemini`, `arkavo-anthropic` (implied), `arkavo-qwen`, etc.
- **Why**: Maintaining separate crates for every provider adds boilerplate. Moving provider implementations into `arkavo-llm` as feature-gated modules (e.g., `features = ["gemini", "qwen"]`) could reduce build graph complexity while keeping binary size low via features.

### 3. TØRG/SBE Ecosystem Consolidation
**Proposal**: `arkavo-sbe`, `arkavo-sat`, `arkavo-torg`, `arkavo-titan`, and `arkavo-ensemble` are all tightly coupled parts of the "Self-Healing/Policy" engine.
- **Why**: These could be organized into a sub-workspace or a single `arkavo-policy-engine` crate. They share common domain types (graphs, tokens, policies) and are rarely used in isolation.

### 4. UI Generation Strategy
**Proposal**: `arkavo-ui-generator`, `arkavo-ui-core`, and `arkavo-agui` have overlapping responsibilities regarding UI state and events.
- **Why**: Clarifying the boundary between "generating code" (`ui-generator`) and "serving/rendering UI" (`agui`, `ui-core`) is important. `arkavo-ui-core` could absorb common types from both to prevent circular dependencies.

### 5. Config & Transport Simplification
**Proposal**: `arkavo-config-bundle`, `arkavo-config-encryption`, and `arkavo-config-transport` are granular.
- **Why**: A single `arkavo-config` crate handling bundles, encryption, and transport logic would reduce API surface area and simplify the "secure config" story for consumers.

### 6. Test Suite Unification
**Proposal**: `arkavo-bench` acts as a benchmark tool, but testing logic is spread across `arkavo-mcp-tools` (test runner) and individual crates.
- **Why**: A dedicated integration testing crate (e.g., `arkavo-tests`) that imports the CLI as a library could provide a standardized way to run end-to-end agent scenarios, complementing unit tests.
