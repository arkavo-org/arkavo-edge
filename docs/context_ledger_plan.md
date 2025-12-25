# Context Ledger Implementation Plan

## Goal
Implement a "Context Ledger" to actively manage the LLM context window by offloading low-value/high-volume text (like large diffs or logs) to local vector storage and replacing them with semantic pointers.

## Architecture

The solution involves four crates:
1.  **`arkavo-memory`**: Storage backend (SQLite + HNSW).
2.  **`arkavo-context`**: Logic for offloading/restoring.
3.  **`arkavo-hrm`**: Strategy definition.
4.  **`arkavo-mcp-tools`**: Tool exposure to LLM.

## Implementation Steps

### Phase 1: Storage (`arkavo-memory`)
- Use the existing `memories` table.
- Define a constant category: `CATEGORY_LEDGER_FRAGMENT = "ledger_fragment"`.
- Add `store_ledger_fragment` helper to `MemoryStorage` in `crates/arkavo-memory/src/lib.rs` (or `storage.rs`).
- Add `get_ledger_fragment` helper.

### Phase 2: Logic (`arkavo-context`)
- Create `crates/arkavo-context/src/ledger.rs`.
- Struct `ContextLedger`.
- Method `offload(content: &str, summary: &str, source: &str) -> Result<String>`:
    - Generates UUID.
    - Stores to `arkavo-memory` with embedding.
    - Returns a "Pointer String" (e.g., `[ARCHIVED: {summary} - ID: {uuid}]`).
- Method `restore(pointer_id: &str) -> Result<String>`:
    - Fetches from `arkavo-memory`.
    - Returns original content.

### Phase 3: Strategy (`arkavo-hrm`)
- Modify `crates/arkavo-hrm/src/burst/contract.rs`.
- Update `ContextStrategy` enum:
    ```rust
    pub enum ContextStrategy {
        // ... existing
        /// Offload large context chunks to local vector store
        Ledger,
    }
    ```
- Update `estimated_overhead_tokens` to return low overhead for `Ledger`.

### Phase 4: Tools (`arkavo-mcp-tools`)
- *Note: `arkavo-mcp-tools` was not fully explored, but assumed to exist based on file structure.*
- Create/Update `crates/arkavo-mcp-tools/src/context_tools.rs`.
- Implement `restore_context(id: String)` tool.
- Register this tool in the main tool registry.

## Usage Workflow
1.  **Trigger**: HRM detects high token usage or task completion.
2.  **Action**: `arkavo-context` scans context, identifies a large block (e.g., "git diff").
3.  **Offload**: Calls `ledger.offload(diff_text, "git diff of main.rs", "git")`.
4.  **Replace**: Replaces text in context with `[ARCHIVED: git diff of main.rs - ID: <uuid>]`.
5.  **Restore**: Later, LLM sees the pointer and calls `restore_context(<uuid>)` if it needs details.
