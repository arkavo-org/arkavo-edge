# The Arkavo Edge Architecture: A Bio-Inspired Design

## The Monolith Problem

Current machine learning architectures treat AI like a brain in a jar: a single, monolithic Transformer network expected to handle language fluency, spatial reasoning, fact retrieval, and logical deduction all at once.

Biology doesn't work this way. The human brain is not a massive, undifferentiated blob of neurons. It is a highly optimized, distributed network of **specialized modules** that communicate over a high-speed routing bus. The brain separates fast reactive processing from slow deliberate planning, and divorces short-term working memory from long-term memory consolidation.

**Arkavo Edge is designed around this biological blueprint.**

We do not build monolithic agents. Arkavo Edge is a secure, sovereign, and self-healing **Cortical Mesh**. By explicitly adopting the architectural principles of neurobiology -- modularity, dual-timescale memory, dynamic routing, and parallel safety circuits -- we achieve highly efficient, multi-agent AI that scales from a Raspberry Pi 5 to cloud-scale orchestrators.

---

## The ML-Brain Blueprint

While the codebase uses standard software terminology (routers, ledgers, orchestrators), the system design maps directly to the functional regions of the biological brain.

```text
+-------------------------------------------------------------+
|                    PREFRONTAL CORTEX                         |
|                arkavo-orchestrator                           |
|  Executive planning, task breakdown, and goal management     |
|  Ephemeral workspace isolation via workspace_container       |
+---------------+--------------+------------------------------+
|   THALAMUS    |  CEREBELLUM  |          AMYGDALA             |
| arkavo-router | arkavo-llama | arkavo-mcp-tools (security)  |
|               | -cpp         |                              |
| mDNS mesh     | llama.cpp    | sec_semgrep (SAST)           |
| Zero-config   | Local model  | sbom_syft (SBOM/CVE)         |
| <0.1ms A2A    | inference     | Fast anomaly detection       |
| routing       |              |                              |
+---------------+------+-------+-----------+------------------+
|                      CORTEX                                  |
|             arkavo-mcp-tools (capabilities)                  |
|                                                              |
|  +-----------+ +----------+ +-----------+ +-----------+      |
|  | codegrep  | | syntax   | | browser   | | test_run  |      |
|  | _search   | | _tree    | | _cdp      | |           |      |
|  | (code)    | | (AST)    | | (vision)  | | (exec)    |      |
|  +-----------+ +----------+ +-----------+ +-----------+      |
+--------------------------------------------------------------+
|                    HIPPOCAMPUS                                |
|                  arkavo-context                               |
|  Active context windowing via CompressionPipeline             |
|  Federated, TDF-protected episodic retrieval (arkavo-tdf)    |
|  Cryptographic access control via OIDC + OpenTDF             |
+--------------------------------------------------------------+
```

---

## The Thalamus: Dynamic Routing via Arkavo Mesh

In the brain, the thalamus is the central relay station. It dynamically routes sensory data to the right specialized regions based on context and attention.

**Implementation:** Arkavo eschews static, hard-coded agent pipelines in favor of a zero-config, dynamic mesh. Using a pure-Rust mDNS implementation (`mdns-sd`), the runtime automatically discovers and connects to other agents on the local network. It routes tasks to the appropriate model provider (local llama.cpp, Gemini, Moonshot, or others) based on cost, capability, and availability. Localhost benchmarks show **<0.1ms round-trip** for agent-to-agent messages over both HTTP and WebSocket transports (`a2a_latency` criterion benchmarks).

**Crates:** `arkavo-router`, `arkavo-protocol` (mDNS, A2A transport), `arkavo-dataflow` (declarative pipelines)

## The Hippocampus: Context Windowing and OpenTDF

The hippocampus handles short-term episodic memory and manages what gets sent to the cortex. Biological memory is heavily permissioned and deeply contextual.

**Implementation:** Arkavo solves the "infinite context window" trap through its **Context Compression Pipeline**. Rather than dumping an entire codebase into an LLM prompt, the pipeline manages an active, rolling window of relevant context using semantic chunking, decomposition, and RLM (Recency-Length-Multimodal) detection. Memory privacy is native via **OpenTDF integration**: retrieval is federated and cryptographically access-controlled through OIDC, meaning the system only "remembers" what the current authorized session permits.

**Crates:** `arkavo-context` (CompressionPipeline, ContextDecomposer, SemanticChunker), `arkavo-tdf` (OpenTDF encryption, KAS client), `arkavo-memory`

## The Cortex: Specialized MCP Toolsets

The biological cortex is divided into specialized regions (Broca's area for language, the visual cortex for sight) that share a common microarchitecture but are tuned for specific modalities.

**Implementation:** Instead of relying on one massive LLM to know how to code, test, and browse, Arkavo delegates capabilities to isolated **Model Context Protocol (MCP)** tools. The LLM acts purely as the reasoning engine, interfacing with specialized "cortical lobes":

| Tool | Modality | What It Does |
|------|----------|--------------|
| `codegrep_search` | Code search | Ripgrep-backed structural code search |
| `syntax_tree` | Logic/AST | Tree-sitter parsing for language-aware analysis |
| `browser_cdp` | Vision/UI | Chrome DevTools Protocol automation |
| `test_run` | Execution | Cross-language test runner (cargo, pytest, jest, go, xcodebuild) |
| `struct_find_replace` | Refactoring | Comby-backed structural find-and-replace |
| `find_bugs` | Analysis | Static code analysis and pattern detection |

**Crates:** `arkavo-mcp-tools`, `arkavo-code-search`, `arkavo-browser`

## The Cerebellum: Local Edge Compute

The cerebellum contains more neurons than the rest of the brain combined. It handles fast, localized, predictive motor control without waiting for the slower prefrontal cortex to process every detail.

**Implementation:** Arkavo is built for the edge. Through native `llama.cpp` integration (via `arkavo-llama-cpp-sys`) and ARM64 builds, Arkavo runs fast, localized inference directly on devices like the Raspberry Pi 5. These local nodes act as the system's cerebellum -- handling low-latency, high-frequency tasks (formatting, fast-path parsing, local sensor abstraction) completely offline.

Recommended edge models:

| Model | Size | Target |
|-------|------|--------|
| Ministral 3B | 2.5 GB (Q5_K_M) | Raspberry Pi 5, 8 GB RAM |
| Ministral 8B | 5.5 GB | Desktop/laptop, 12 GB VRAM |
| Qwen3 0.6B | ~640 MB | Ultra-constrained devices |

**Crates:** `arkavo-llama-cpp`, `arkavo-llama-cpp-sys`, `arkavo-llm` (provider abstraction)

## The Prefrontal Cortex: Orchestration and Workspaces

The prefrontal cortex (PFC) manages executive functions: planning, maintaining multiple goal states, inhibition, and delegating sub-tasks.

**Implementation:** The `arkavo-orchestrator` acts as the PFC. It sits above the mesh, polling external environments (GitHub webhooks), autonomously classifying requirements, breaking them into sub-tasks, and dispatching them to specialized agents. To ensure plans are executed safely, the orchestrator utilizes `workspace_container` to spawn isolated Docker/Podman environments with strict resource quotas -- effectively "inhibiting" agents from impacting the host OS.

**Crates:** `arkavo-orchestrator` (GitHub webhook orchestration, agent assignment), `arkavo-workspace` (container isolation)

## The Amygdala: Parallel Safety Circuits

The amygdala processes threats fast, operating concurrently with slower reasoning circuits to hijack the system if danger is detected.

**Implementation:** Safety cannot be a system prompt that an LLM can "forget." Arkavo utilizes hard-coded, parallel security tools that run alongside reasoning steps as an always-on guardrail:

| Tool | Function |
|------|----------|
| `sec_semgrep` | SAST scanning via Semgrep |
| `sbom_syft` | SBOM generation and dependency vulnerability checks via Syft |
| Egress filter | IP-level SSRF prevention blocking metadata endpoints and private ranges |
| Input validation | Unicode normalization, path traversal prevention, injection blocking |

These tools can immediately flag or block unsafe code modifications before they are ever committed.

**Crates:** `arkavo-mcp-tools` (semgrep, syft wrappers), `arkavo-validation` (input sanitization), `arkavo-protocol` (egress filtering)

---

## The Road Ahead: Memory Consolidation

By defining the architecture against this biological blueprint, the gaps become the product roadmap.

The biggest missing piece in modern AI -- and Arkavo's next frontier -- is **hippocampal consolidation** (the basal ganglia / sleep loop). Currently, agents learn *in-context* but forget when the session ends.

The router already has a **learning module** (`arkavo-router/src/learning/`) that tracks agent utility and coordination metrics at runtime. The next step is offline consolidation: background daemons that activate during idle compute to extract successful execution traces from the context pipeline, distill them into training signal, and continuously improve local edge model selection and routing decisions.

**Arkavo isn't just a router for LLMs. It is the operating system for a specialized, distributed, synthetic brain.**
