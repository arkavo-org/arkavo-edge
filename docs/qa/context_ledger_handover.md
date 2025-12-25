# Context Ledger QA Handover

**Feature:** Active Context Ledger (ACM)
**Component:** `arkavo-hrm`, `arkavo-memory`, `arkavo-mcp-tools`
**Version:** 1.0.0

## 1. Feature Overview
The Active Context Ledger allows the Arkavo runtime to autonomous offload large text blocks (logs, git diffs) from the active context window into local vector storage. It replaces the raw text with a semantic pointer (`[ARCHIVED: Summary - ID: <uuid>]`) to save tokens. The agent can restore this context on-demand using the `context_restore` tool.

## 2. Architecture & Impact Area

| Component | Change Scope | Risk Level |
| :--- | :--- | :--- |
| **`arkavo-memory`** | Added `ContextLedger` logic and `store_ledger_fragment` storage methods. | Low (Additive) |
| **`arkavo-hrm`** | Updated `Conductor` to support `ContextStrategy::Ledger`. Added `prepare_context_for_burst`. | High (Core Logic) |
| **`arkavo-mcp-tools`** | Added `ContextRestoreTool`. | Medium |

## 3. Test Scenarios

### 3.1 Automated Verification
A comprehensive integration test suite has been added. Run this to verify baseline functionality.

```bash
cargo test -p arkavo-hrm --test comprehensive_ledger_test
```

**Key Assertions Verified:**
*   **Compression:** Input > 10KB results in Output < 200 bytes.
*   **Integrity:** Restored text matches original text bit-for-bit.
*   **Strategy:** Passing `ContextStrategy::Full` does *not* archive text.
*   **Tool Isolation:** `ContextRestoreTool::with_path()` enables test DB isolation.

### 3.2 Manual Verification: The "Noise Bomb"
A simulation script generates a massive compiler error log to test the system's stability under load.

**Steps:**
1.  Navigate to `examples/autonomous_refactor`.
2.  Run `./run_demo.sh`.
3.  Observe the generated "Noise Bomb" (compiler errors).
4.  Execute Arkavo against this task (command provided by script).

**Success Criteria:**
*   Agent does not crash or timeout.
*   Logs show `[ARCHIVED: ...]` pointers instead of raw error dumps.
*   Agent successfully identifies the specific service error despite the noise.

### 3.3 Metrics Validation
To validate the performance claims (Reduction Ratio, Cost Savings), execute the metrics suite:

```bash
cargo test -p arkavo-hrm --test paper_metrics_test -- --nocapture
```

**Expected Baselines:**
*   Noise Removal: > 99.0%
*   Processing Latency: < 50ms

## 4. Known Limitations & Failure Modes

### 4.1 Embedding Dependency
**Risk:** Retrieval relies on the local embedding model (`AllMiniLML6V2`).
**Failure Mode:** If the ONNX model files are missing or the runtime fails to initialize, Semantic Search precision drops to **0%**.
**QA Action:** Verify that `arkavo` binaries are distributed with the correct `models/` directory structure. Test in a clean environment (e.g., Docker container) to ensure model loading works.

### 4.2 Context Thrashing
**Risk:** High frequency of Restore operations.
**Threshold:** If the agent restores data more than **once every 5 steps (20%)**, the operation becomes more expensive than standard context retention.
**QA Action:** Monitor token usage logs. If `output_tokens` spike significantly during "Restore" operations, flag as a potential regression in agent strategy behavior.

### 4.3 Context Blindness
**Risk:** The agent cannot "see" offloaded data until restored.
**Impact:** Agents may hallucinate details about archived code if they fail to call `context_restore`.
**QA Action:** Verify that the prompt instructions clearly encourage the agent to restore context before answering specific questions about archived data.

## 5. Debugging
When debugging context issues, look for the following trace logs:

*   `prepare_context_for_burst`: Indicates the Conductor is deciding whether to offload.
*   `store_ledger_fragment`: Indicates successful storage in SQLite.
*   `SearchResult`: Logs from the HNSW vector search during restoration.
