# SwarmKit PR-review — WS-C (least-privilege specialization) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Realize the manifest's per-role MCP-tool grants as an enforced least-privilege boundary on a specialized agent. On `agent.specialize`, persist the persona's granted tool names into `AgentMetadata`; build the agent's `ToolRegistry` containing ONLY the granted tools; reject an ungranted tool call before it executes; and drive the specialized agent to actually run its role's task through the conductor loop. A reviewer-role agent that tries `github_pr_create` is denied (negative security test).

**Architecture:** Grants are bare tool names (e.g. `git_diff`, `gh_pr_review`) that match `ToolRegistry` keys 1:1 — the manifest's `McpToolGrant.server` is a *logical label* (`arkavo-github`, `arkavo-git`) and is **never** used for matching (confirmed against `examples/github-ops-kit/github-ops-kit.swarmkit.yaml` and the live registry keys in `registry.rs::register_all`). Filtering is a pure method on `ToolRegistry` (`retain_granted`) so it is unit-testable and reusable. The agent loop builds its registry once at startup, then re-derives a filtered registry whenever the persisted grant set changes (memoized by a grant hash, so no per-cycle rebuild churn). Call-time enforcement is defense-in-depth inside `execute_tool_calls`: even if a filtered registry somehow exposed a tool, an ungranted name is rejected pre-execution. Post-specialize role execution reuses the existing `agent_event_tx` channel (the same path A2A messages use to enter the loop) to inject the role's task once.

**Tech Stack:** Rust, `arkavo-server` (`agent_loop.rs`, `conductor.rs`, `conductor_tool_loop.rs`, `handlers/specialization.rs`, `config_helpers.rs`), `arkavo-mcp-tools` (`ToolRegistry`), `arkavo-protocol` (`AgentSpecializationBundle`, `RoleContext`). All in-process; no new crate dependencies.

## Global Constraints

- No `--release` builds; use debug.
- No clippy warnings: `cargo clippy -p arkavo-mcp-tools -- -D warnings` and `cargo clippy -p arkavo-server -- -D warnings`. `#[allow(dead_code)]` forbidden.
- Implementation code (excluding `#[cfg(test)]` modules) stays under 400 lines per file. `agent_loop.rs` is already ~624 lines incl. tests but ~330 lines of impl; the WS-C additions must not push impl over 400 — extract the filtered-registry derivation into a small free function (`derive_filtered_registry`) rather than inlining a large block. `specialization.rs` impl is ~205 lines; the WS-C addition there is a few lines (persist + helper) — keep it minimal.
- No new crate dependencies. If `Cargo.toml` changes, commit `Cargo.lock` (no new deps expected — `HashSet` is `std`).
- No Conventional Commits prefixes. Use the exact commit messages below incl. their `Co-Authored-By` / `Claude-Session` trailers.
- Tests must not hit the network or load a real model — test the pure filter, the metadata-persist path (stub decryptor), and the enforcement gate with a fake tool; the role-execution injection is asserted against the channel, not a live conductor run.

## Cross-workstream coordination (READ — WS-D also edits `specialization.rs`)

WS-D rewrites the *front* of `specialization.rs::handle_inner` to fetch the bundle blob from an iroh ticket (`IrohTransport.fetch_bytes`) before `unwrap_bundle`. To avoid a conflict:

- **WS-C touches ONLY `apply_bundle_to_metadata` (specialization.rs:189-203) plus one new helper `granted_tool_names(bundle)` and one call site inside it.** WS-C does NOT touch `handle_inner`'s decode/decrypt prologue (lines 88-152), the `activated` computation (154-164), or the response construction (178-186) beyond reading the already-persisted grants. The post-specialize role-execution injection is added as a *separate* call after `apply_bundle_to_metadata`/`role_specialization.set` (around line 167) — a single new function call, kept to one localized block.
- WS-C adds two new params to `handle_agent_specialize` / `handle_inner` (`agent_event_tx`, `granted_role_task` plumbing) — see Task 4. These are appended to the signature; WS-D's ticket-fetch change is in the body prologue and will not collide if both keep edits localized.

## File Structure

- `crates/arkavo-mcp-tools/src/registry.rs` — add pure `ToolRegistry::retain_granted(&mut self, granted: &HashSet<String>)` + inline `#[cfg(test)]` tests. (~25 impl lines.)
- `crates/arkavo-server/src/server/config_helpers.rs` — add `pub granted_tools: Vec<String>` field to `AgentMetadata`.
- `crates/arkavo-server/src/server/handlers/specialization.rs` — persist grants into `AgentMetadata.granted_tools` (in `apply_bundle_to_metadata`); add `granted_tool_names()` helper; add the post-specialize role-task injection.
- `crates/arkavo-server/src/server/agent_loop.rs` — add `agent_metadata` to `AgentLoopConfig`; add free fn `derive_filtered_registry`; in the tick branch, re-derive the filtered registry when the grant set changes; pass the granted set to the conductor.
- `crates/arkavo-server/src/server/conductor.rs` — thread an `Option<&std::collections::HashSet<String>>` granted set through `execute_with_conductor_and_learning` into the tool loop.
- `crates/arkavo-server/src/server/conductor_tool_loop.rs` — accept the granted set; reject ungranted tool calls in `execute_tool_calls` before execution.
- `crates/arkavo-server/src/server/a2a_server.rs` — pass `agent_metadata` into `AgentLoopConfig` (one field), and `agent_event_tx` into the specialize handler wiring (Task 4).
- `crates/arkavo-server/src/server/mod.rs` — pass `&self.agent_event_tx` to `handle_agent_specialize`.

---

### Task 1: Pure grant filter on `ToolRegistry`

**Files:**
- Modify: `crates/arkavo-mcp-tools/src/registry.rs`
- Test: inline `#[cfg(test)]` in `registry.rs`

**Interfaces:**
- Consumes (existing): `ToolRegistry { tools: HashMap<String, Box<dyn Tool>> }`, `ToolRegistry::register`, `ToolRegistry::list_tools`, the `Tool`/`ToolSchema` traits.
- Produces: `pub fn retain_granted(&mut self, granted: &std::collections::HashSet<String>)` — drops every tool whose registry key is not in `granted`. Matching is by registry key (bare tool name), NOT by `server:tool`.

- [ ] **Step 1: Read the existing pattern**

Read `crates/arkavo-mcp-tools/src/registry.rs` around the `ToolRegistry` struct (`tools: HashMap<String, Box<dyn Tool>>`, ~line 218) and `register`/`get`/`list_tools` (lines 343-410) so the new method uses the same `HashMap` field directly.

- [ ] **Step 2: Write the failing test**

Add to a `#[cfg(test)]` module in `registry.rs` (one already exists at the bottom — append a new module or reuse it; check first):

```rust
#[cfg(test)]
mod retain_granted_tests {
    use super::*;
    use arkavo_memory::MemoryStorage;
    use std::collections::HashSet;
    use std::sync::Arc;

    fn names(reg: &ToolRegistry) -> Vec<String> {
        let mut n: Vec<String> = reg.list_tools().into_iter().map(|t| t.name).collect();
        n.sort();
        n
    }

    #[tokio::test]
    async fn retains_only_granted_tools() {
        // `new` registers the full built-in catalog incl. git_diff,
        // gh_pr_review, github_pr_create.
        let storage = Arc::new(MemoryStorage::new_test().await.expect("storage"));
        let mut reg = ToolRegistry::new(storage);
        assert!(reg.get("github_pr_create").is_some(), "precondition: full catalog");

        let granted: HashSet<String> =
            ["git_diff", "git_log", "gh_pr_review", "github_ci_status"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        reg.retain_granted(&granted);

        let kept = names(&reg);
        assert_eq!(kept, vec!["gh_pr_review", "git_diff", "git_log", "github_ci_status"]);
        // The ungranted write tool is gone — least-privilege boundary.
        assert!(reg.get("github_pr_create").is_none());
    }

    #[tokio::test]
    async fn empty_grant_set_clears_registry() {
        let storage = Arc::new(MemoryStorage::new_test().await.expect("storage"));
        let mut reg = ToolRegistry::new(storage);
        reg.retain_granted(&HashSet::new());
        assert!(reg.list_tools().is_empty());
    }

    #[tokio::test]
    async fn grant_for_absent_tool_is_a_noop_not_an_insert() {
        let storage = Arc::new(MemoryStorage::new_test().await.expect("storage"));
        let mut reg = ToolRegistry::new(storage);
        let granted: HashSet<String> = ["does_not_exist".to_string()].into_iter().collect();
        reg.retain_granted(&granted);
        // Granting a tool the agent doesn't have must NOT fabricate it.
        assert!(reg.get("does_not_exist").is_none());
        assert!(reg.list_tools().is_empty());
    }
}
```

> `MemoryStorage` has no sync constructor; the crate's tests use the async `MemoryStorage::new_test().await` (verified in `registry.rs` tests ~line 772). Hence `#[tokio::test]`. If `new_test` is gated/renamed, match the constructor the nearest existing `registry.rs` test uses.

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p arkavo-mcp-tools retain_granted_tests`
Expected: FAIL to compile — `retain_granted` does not exist.

- [ ] **Step 4: Implement `retain_granted`**

Add to `impl ToolRegistry` (near `register`, ~line 343):

```rust
    /// Drop every tool whose registry key is not in `granted`, realizing a
    /// SwarmKit role's least-privilege MCP-tool grant. Matching is by the
    /// registry key (the bare tool name, e.g. `git_diff`) — the manifest's
    /// `McpToolGrant.server` is a logical label and is never matched here.
    /// Granting a tool the agent does not have is a no-op (never fabricates).
    pub fn retain_granted(&mut self, granted: &std::collections::HashSet<String>) {
        self.tools.retain(|name, _| granted.contains(name));
    }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p arkavo-mcp-tools retain_granted_tests`
Expected: PASS (3 tests).

- [ ] **Step 6: Build + clippy**

Run: `cargo build -p arkavo-mcp-tools` (clean)
Run: `cargo clippy -p arkavo-mcp-tools -- -D warnings` (clean)

- [ ] **Step 7: Commit**

```bash
git add crates/arkavo-mcp-tools/src/registry.rs
git commit -m "Add ToolRegistry::retain_granted for least-privilege filtering

Pure method that drops every tool whose registry key is not in the granted
set. Matching is by bare tool name (the registry key) — the manifest's
McpToolGrant.server is a logical label, never matched. Granting an absent
tool is a no-op (never fabricates a tool the agent lacks). Unit-tested
against the full built-in catalog.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task 2: Persist `persona.mcp_tools` in `AgentMetadata`

**Files:**
- Modify: `crates/arkavo-server/src/server/config_helpers.rs`
- Modify: `crates/arkavo-server/src/server/handlers/specialization.rs`
- Test: extend the existing `#[cfg(test)]` module in `specialization.rs`

**Interfaces:**
- Consumes (existing): `AgentMetadata` (config_helpers.rs:38-53, `#[derive(Debug, Clone, Default)]`), `AgentSpecializationBundle.persona.mcp_tools: Vec<McpToolGrant>` where `McpToolGrant { server: String, tools: Vec<String> }`, `apply_bundle_to_metadata` (specialization.rs:189-203).
- Produces: `AgentMetadata.granted_tools: Vec<String>` (bare tool names, deduped, sorted for determinism); a helper `granted_tool_names(bundle) -> Vec<String>`.

- [ ] **Step 1: Read the persist path**

Re-read `specialization.rs:154-203` — note `apply_bundle_to_metadata` already copies `purpose`/`model`/`api_keys`/`name`; the `activated` vec (lines 154-164) builds `server:tool` strings for the RPC *response* only. The registry filter needs *bare* tool names, so do NOT reuse `activated`.

- [ ] **Step 2: Write the failing test**

Extend `specialization.rs`'s test module. The existing `build_bundle` grants `McpToolGrant { server: "asset-store", tools: ["read", "describe"] }`. Add:

```rust
    #[tokio::test]
    async fn specialize_persists_bare_granted_tool_names() {
        let did = "did:web:agent-7.arkavo.net";
        let bundle = build_bundle("analyst", "agent-7");
        let agent_metadata = metadata_with_did("agent-7", did);
        let (metrics, limiter, registry, role_store) = deps();
        let decryptor = StubDecryptor {
            bundle: bundle.clone(),
            expected_did: did.to_string(),
        };

        handle_agent_specialize(
            &metrics, &limiter, &registry, &agent_metadata, &role_store,
            &decryptor,
            AgentSpecializeRequest {
                requester_id: "did:web:orchestrator.arkavo.net".into(),
                encrypted_bundle: encoded_dummy_bytes(),
                task_context: None,
                session_id: None,
            },
        )
        .await
        .expect("specialize");

        let meta = agent_metadata.read().await;
        // Bare tool names (NOT "asset-store:read") so they match registry keys.
        assert_eq!(meta.granted_tools, vec!["describe".to_string(), "read".to_string()]);
    }
```

> If Task 4 has already extended `handle_agent_specialize`'s signature with the event-tx param, add the new argument here too (pass `None`). Match whatever the current signature is.

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p arkavo-server --lib specialize_persists_bare_granted_tool_names`
Expected: FAIL to compile — `granted_tools` field does not exist.

- [ ] **Step 4: Add the field to `AgentMetadata`**

In `config_helpers.rs`, in the `AgentMetadata` struct (after `delegated_entitlements`, ~line 52):

```rust
    /// Bare MCP tool names this agent is granted by its SwarmKit role
    /// (from `persona.mcp_tools`). Empty for an unspecialized agent (no
    /// filtering applied). When non-empty, the agent loop filters its
    /// `ToolRegistry` to exactly this set — least-privilege (design D9).
    pub granted_tools: Vec<String>,
```

`#[derive(Default)]` covers the empty default; no other change needed.

- [ ] **Step 5: Add the helper + persist in `apply_bundle_to_metadata`**

In `specialization.rs`, add a free helper near `apply_bundle_to_metadata`:

```rust
/// Flatten a persona's `mcp_tools` grants into the deduped, sorted set of
/// bare tool names. These are registry keys (e.g. `git_diff`) — the grant's
/// `server` field is a logical label and is intentionally dropped here.
fn granted_tool_names(bundle: &AgentSpecializationBundle) -> Vec<String> {
    let mut names: Vec<String> = bundle
        .persona
        .mcp_tools
        .iter()
        .flat_map(|grant| grant.tools.iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    names
}
```

Then, inside `apply_bundle_to_metadata` (after the `api_keys` clone, ~line 196):

```rust
    meta.granted_tools = granted_tool_names(bundle);
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p arkavo-server --lib specialize_persists_bare_granted_tool_names`
Expected: PASS. Also re-run the existing handler tests:
Run: `cargo test -p arkavo-server --lib handle_specialize`
Expected: PASS (4 pre-existing tests unaffected).

- [ ] **Step 7: Build + clippy**

Run: `cargo build -p arkavo-server` (clean)
Run: `cargo clippy -p arkavo-server -- -D warnings` (clean)

- [ ] **Step 8: Commit**

```bash
git add crates/arkavo-server/src/server/config_helpers.rs \
        crates/arkavo-server/src/server/handlers/specialization.rs
git commit -m "Persist persona MCP-tool grants in AgentMetadata on specialize

agent.specialize now records the role's bare granted tool names in
AgentMetadata.granted_tools (deduped, sorted) so the agent loop can filter
its ToolRegistry to exactly that set. Server labels from McpToolGrant are
dropped — grants match registry keys. Change is localized to
apply_bundle_to_metadata to avoid conflict with WS-D's ticket-fetch edit.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task 3: Grant-filter the agent's registry + call-time enforcement

**Files:**
- Modify: `crates/arkavo-server/src/server/agent_loop.rs`
- Modify: `crates/arkavo-server/src/server/conductor.rs`
- Modify: `crates/arkavo-server/src/server/conductor_tool_loop.rs`
- Modify: `crates/arkavo-server/src/server/a2a_server.rs` (one field on `AgentLoopConfig`)
- Test: inline `#[cfg(test)]` in `conductor_tool_loop.rs` (enforcement gate) + `agent_loop.rs` (filter derivation)

**Interfaces:**
- Consumes (existing): `AgentLoopConfig` (agent_loop.rs:15-38), the cached-registry build (agent_loop.rs:118-143), `execute_with_conductor_and_learning` (conductor.rs:68-86, already `#[allow(clippy::too_many_arguments)]`), `run_tool_loop` (conductor_tool_loop.rs:43-53), `execute_tool_calls` (conductor_tool_loop.rs:640-653), `ToolRegistry::retain_granted` (Task 1), `AgentMetadata.granted_tools` (Task 2).
- Produces: free fn `derive_filtered_registry(base: &Arc<ToolRegistry>, granted: &HashSet<String>) -> Arc<ToolRegistry>`; a per-loop memoized filtered registry keyed on a grant hash; `Option<&HashSet<String>>` granted-set param threaded `agent_loop → conductor → run_tool_loop → execute_tool_calls`; a pre-execution rejection in `execute_tool_calls`.

> **Note on the cached registry:** `ToolRegistry` is not `Clone` and the loop holds it as `Arc<ToolRegistry>` (cannot mutate in place). `derive_filtered_registry` therefore **rebuilds** an empty registry and re-registers only the granted tools by re-running the same registration the loop's startup block does, then calls `retain_granted`. Simplest correct approach: rebuild via the same code path as agent_loop.rs:118-143 wrapped in a helper, then filter. See Step 4.

- [ ] **Step 1: Write the failing enforcement test (conductor_tool_loop.rs)**

The cleanest unit-testable seam is the pre-execution check. Add a small pure predicate and test it. In `conductor_tool_loop.rs`, plan a helper:

```rust
/// True if a tool call is permitted under an optional grant set.
/// `None` = unspecialized agent (no filtering). `Some(set)` = specialized;
/// only names in the set may execute.
fn tool_call_permitted(name: &str, granted: Option<&std::collections::HashSet<String>>) -> bool {
    granted.is_none_or(|g| g.contains(name))
}
```

Add the test (inline `#[cfg(test)] mod tests`):

```rust
    #[spec("SRV-009")]
    #[test]
    fn ungranted_tool_call_is_denied() {
        use std::collections::HashSet;
        let granted: HashSet<String> =
            ["git_diff", "gh_pr_review"].iter().map(|s| s.to_string()).collect();
        // reviewer role: may diff + review, must NOT create PRs
        assert!(tool_call_permitted("git_diff", Some(&granted)));
        assert!(tool_call_permitted("gh_pr_review", Some(&granted)));
        assert!(!tool_call_permitted("github_pr_create", Some(&granted)));
    }

    #[spec("SRV-009")]
    #[test]
    fn no_grant_set_permits_everything() {
        assert!(tool_call_permitted("anything", None));
    }
```

Run: `cargo test -p arkavo-server --lib ungranted_tool_call_is_denied`
Expected: FAIL to compile — `tool_call_permitted` does not exist.

- [ ] **Step 2: Implement the predicate + the gate in `execute_tool_calls`**

Add `tool_call_permitted` (above). Then thread a granted set into `run_tool_loop` and `execute_tool_calls`:

In `run_tool_loop` signature (conductor_tool_loop.rs:43-53), append:
```rust
    granted_tools: Option<&std::collections::HashSet<String>>,
```
Pass it into the `execute_tool_calls(...)` call (conductor_tool_loop.rs:463-477) as a new final argument.

In `execute_tool_calls` signature (conductor_tool_loop.rs:640-653), append the same param. At the top of the `for tool_call in tool_calls` loop (after the setup-tool skip, before building `args`, ~line 671), add:

```rust
        if !tool_call_permitted(&tool_call.tool_name, granted_tools) {
            warn!(
                tool = %tool_call.tool_name,
                "Tool call denied: not in this agent's granted set (least-privilege)"
            );
            tool_result_parts.push(format!(
                "Tool {} (Denied): not permitted for this role — \
                 it is not in the agent's granted tool set.",
                tool_call.tool_name
            ));
            *total_step_idx += 1;
            continue;
        }
```

> This is defense-in-depth: the registry is already filtered (Step 4), so a granted-only model never sees the tool. The gate catches a model that hallucinates an ungranted tool name (which `registry_arc.get` would miss, falling through to `mcp_registry.call_tool` at line 684 — exactly the hole this closes).

- [ ] **Step 3: Run the enforcement tests**

Run: `cargo test -p arkavo-server --lib tool_call_permitted ungranted_tool_call_is_denied no_grant_set_permits_everything`
Expected: PASS (2 tests). Also confirm the existing `conductor_tool_loop` tests still pass:
Run: `cargo test -p arkavo-server --lib conductor_tool_loop`
Expected: PASS (no regressions).

- [ ] **Step 4: Thread the granted set + filtered registry through the loop**

`conductor.rs`: add a param to `execute_with_conductor_and_learning` (conductor.rs:68-86) — append after `cached_registry`:
```rust
    granted_tools: Option<&std::collections::HashSet<String>>,
```
Forward it to BOTH loop call sites (conductor.rs:463 parallel + 476 single). The parallel loop (`conductor_parallel::run_tool_loop_parallel`) must accept and forward it too — add the same param there and into its internal `execute_tool_calls` calls (grep `execute_tool_calls` in `conductor_parallel.rs`; if that file calls a shared `execute_tool_calls`, the param is already added in Step 2 — just forward `granted_tools`).
Update the legacy `execute_with_conductor` wrapper (conductor.rs:31-60) to pass `None`.

`agent_loop.rs`:
- Add to `AgentLoopConfig` (after `iroh_node` / before close, ~line 38):
```rust
    /// Shared agent metadata — read each cycle for the SwarmKit grant set
    /// so the registry can be filtered after specialization (design D9).
    pub agent_metadata: Arc<tokio::sync::RwLock<crate::server::config_helpers::AgentMetadata>>,
```
- Extract the startup registry build (lines 118-143) into an `async` free fn returning an **owned** `ToolRegistry` (not `Arc` — `ToolRegistry` is not `Clone`, so the owned value is what `retain_granted` mutates and the startup site wraps in `Arc::new`). `config.mcp_registry.list_all_tools()` is `async` so the builder is `async fn`; mesh/iroh registration is sync within it. Both the startup site and the per-cycle re-derivation run in async context, so `.await` is fine.

Final shape (use exactly this):
```rust
async fn build_full_registry(config: &AgentLoopConfig) -> arkavo_mcp_tools::ToolRegistry {
    use arkavo_mcp_tools::ToolRegistry;
    let mut registry = ToolRegistry::empty();
    if let Ok(mcp_tools) = config.mcp_registry.list_all_tools().await {
        for tool in mcp_tools {
            let tool_name = tool.name.clone();
            let bridge = super::mcp_bridge::McpBridgeTool::new(config.mcp_registry.clone(), tool);
            registry.register(&tool_name, Box::new(bridge));
        }
    }
    arkavo_mcp_mesh::register_tools(&mut registry, config.mesh_state.clone());
    #[cfg(feature = "iroh")]
    if let Some(ref node) = config.iroh_node {
        arkavo_mcp_tools::iroh_data::register_iroh_tools(&mut registry, node.clone());
    }
    registry
}

async fn derive_filtered_registry(
    config: &AgentLoopConfig,
    granted: &std::collections::HashSet<String>,
) -> Arc<arkavo_mcp_tools::ToolRegistry> {
    let mut registry = build_full_registry(config).await;
    registry.retain_granted(granted);
    Arc::new(registry)
}
```

Replace the startup block (agent_loop.rs:118-143) with:
```rust
    let full_registry = build_full_registry(&config).await;
    info!("Agent loop: cached {} tools for reuse across cycles", full_registry.list_tools().len());
    let mut active_registry = Arc::new(full_registry);
    let mut active_grant_hash: u64 = 0; // 0 = unfiltered
```

In the tick branch, BEFORE assembling the cycle prompt (after the budget gate, ~after line 185), add the re-derivation:
```rust
                // Re-derive the registry to the agent's granted tool set when
                // specialization has set/changed it. Memoized on a grant hash
                // so this is not per-cycle work.
                let granted_set: std::collections::HashSet<String> = {
                    let meta = config.agent_metadata.read().await;
                    meta.granted_tools.iter().cloned().collect()
                };
                let grant_hash = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    let mut sorted: Vec<&String> = granted_set.iter().collect();
                    sorted.sort();
                    sorted.hash(&mut h);
                    h.finish()
                };
                if !granted_set.is_empty() && grant_hash != active_grant_hash {
                    active_registry = derive_filtered_registry(&config, &granted_set).await;
                    active_grant_hash = grant_hash;
                    info!(
                        granted = granted_set.len(),
                        tools = active_registry.list_tools().len(),
                        "Specialized: registry filtered to granted tool set"
                    );
                }
```

Change the conductor call (agent_loop.rs:386) from `Some(cached_registry.clone())` to `Some(active_registry.clone())`, and add the granted-set argument:
```rust
                    Some(active_registry.clone()),
                    if granted_set.is_empty() { None } else { Some(&granted_set) },
                    #[cfg(feature = "iroh")]
                    config.iroh_node.as_ref(),
```
> The `granted_tools` param position in `execute_with_conductor_and_learning` is *after* `cached_registry` and *before* the `#[cfg(feature="iroh")] iroh_node` — match Task 3 Step 4's signature edit exactly.

`a2a_server.rs`: in the `AgentLoopConfig { ... }` construction (a2a_server.rs:1371), add one field:
```rust
            agent_metadata: self.agent_metadata.clone(),
```
(`self.agent_metadata` is already an `Arc<RwLock<AgentMetadata>>` — confirmed a2a_server.rs:37.)

- [ ] **Step 5: Add the filter-derivation unit test (agent_loop.rs)**

`build_full_registry` needs a live `McpRegistry`/`MeshToolsState`; a full integration test is heavy. Instead assert the *composition* the loop relies on with a focused test that filters a known registry — this is already covered by Task 1's `retain_granted_tests`. Add one agent_loop test that the grant-hash memo logic is order-independent:

```rust
#[cfg(test)]
mod grant_hash_tests {
    use std::collections::HashSet;
    use std::hash::{Hash, Hasher};

    fn hash_of(set: &HashSet<String>) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut sorted: Vec<&String> = set.iter().collect();
        sorted.sort();
        sorted.hash(&mut h);
        h.finish()
    }

    #[test]
    fn grant_hash_is_order_independent() {
        let a: HashSet<String> = ["git_diff", "gh_pr_review"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["gh_pr_review", "git_diff"].iter().map(|s| s.to_string()).collect();
        assert_eq!(hash_of(&a), hash_of(&b));
    }

    #[test]
    fn different_grants_differ() {
        let a: HashSet<String> = ["git_diff"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["github_pr_create"].iter().map(|s| s.to_string()).collect();
        assert_ne!(hash_of(&a), hash_of(&b));
    }
}
```

Run: `cargo test -p arkavo-server --lib grant_hash_tests`
Expected: PASS (2 tests).

- [ ] **Step 6: Build, full server test, clippy**

Run: `cargo build -p arkavo-server` (clean — verify all call sites of `execute_with_conductor_and_learning` and `run_tool_loop` updated, incl. `conductor_parallel.rs` and `agent_loop.rs`)
Run: `cargo test -p arkavo-server --lib` (PASS — no regressions)
Run: `cargo clippy -p arkavo-server -- -D warnings` (clean)

- [ ] **Step 7: Commit**

```bash
git add crates/arkavo-server/src/server/agent_loop.rs \
        crates/arkavo-server/src/server/conductor.rs \
        crates/arkavo-server/src/server/conductor_tool_loop.rs \
        crates/arkavo-server/src/server/conductor_parallel.rs \
        crates/arkavo-server/src/server/a2a_server.rs
git commit -m "Filter specialized agent registry to granted tools + deny at call time

The agent loop now re-derives its ToolRegistry to exactly AgentMetadata.
granted_tools once specialization sets it (memoized on a grant hash, so no
per-cycle rebuild). execute_tool_calls additionally rejects any tool call
whose name is outside the grant set before execution — defense-in-depth
against a hallucinated ungranted tool name falling through to the MCP
registry. Unspecialized agents (empty grant set) are unaffected.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task 4: Post-specialize role execution

**Files:**
- Modify: `crates/arkavo-server/src/server/handlers/specialization.rs`
- Modify: `crates/arkavo-server/src/server/mod.rs` (pass `agent_event_tx`)
- Test: extend `specialization.rs` test module

**Interfaces:**
- Consumes (existing): `agent_event_tx: Arc<tokio::sync::Mutex<Option<mpsc::Sender<AgentEvent>>>>` (mod.rs A2aRpcImpl field; a2a_server.rs:82,1369), `AgentEvent::IncomingMessage { sender, content, task_id, correlation_id, reply }` (agent_event.rs), `RoleContext` (role_id/role_type/handoff_targets) + `AgentPersona.purpose`.
- Produces: after `apply_bundle_to_metadata` + `role_specialization.set`, the handler builds a role-task string and pushes it into the loop via `agent_event_tx`.

> **Design choice (flag — see report):** the role's *procedure* is emergent (design D6) — the manifest gives purpose + grants, not a fixed routine. So the injected task is a kickoff prompt derived from `persona.purpose` + `role_context.role_id`/`role_type` + handoff targets, NOT a hard-coded routine. The agent's continuous loop then drives the work with its now-filtered tools. We inject ONE kickoff message; the loop's existing tick cadence + mesh state take over from there.

- [ ] **Step 1: Write the failing test**

Add to `specialization.rs` tests — assert that a kickoff message is delivered to the channel:

```rust
    #[tokio::test]
    async fn specialize_injects_role_kickoff_task() {
        use crate::server::agent_event::AgentEvent;
        let did = "did:web:agent-7.arkavo.net";
        let bundle = build_bundle("pr_reviewer", "agent-7");
        let agent_metadata = metadata_with_did("agent-7", did);
        let (metrics, limiter, registry, role_store) = deps();
        let decryptor = StubDecryptor { bundle, expected_did: did.to_string() };

        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(4);
        let event_tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

        handle_agent_specialize(
            &metrics, &limiter, &registry, &agent_metadata, &role_store,
            &decryptor, &event_tx,
            AgentSpecializeRequest {
                requester_id: "did:web:orchestrator.arkavo.net".into(),
                encrypted_bundle: encoded_dummy_bytes(),
                task_context: None,
                session_id: None,
            },
        )
        .await
        .expect("specialize");

        let evt = rx.try_recv().expect("kickoff event injected");
        match evt {
            AgentEvent::IncomingMessage { content, .. } => {
                assert!(content.contains("pr_reviewer"), "kickoff names the role: {content}");
            }
            other => panic!("expected IncomingMessage, got {other:?}"),
        }
    }
```

> The existing handler tests call `handle_agent_specialize` with 7 positional args; this adds an 8th (`&event_tx`). Update the four existing tests to pass a dummy `&Arc::new(tokio::sync::Mutex::new(None))` (no channel wired) so they still compile.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p arkavo-server --lib specialize_injects_role_kickoff_task`
Expected: FAIL to compile — `handle_agent_specialize` takes no `event_tx` param yet.

- [ ] **Step 3: Add the param + injection**

`specialization.rs`:
- Add `use crate::server::agent_event::{AgentEvent, CorrelationId};` (confirm the `CorrelationId` constructor/shape in `agent_event.rs`; `IncomingMessage.reply` is a `oneshot::Sender<CycleReceipt>` — for an injected kickoff with no reply path, see Step 3a).
- Append a param to `handle_agent_specialize` AND `handle_inner`:
```rust
    agent_event_tx: &Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::Sender<AgentEvent>>>>,
```
- After `role_specialization.set(bundle.role_context.clone()).await;` (specialization.rs:167), add:
```rust
    inject_role_kickoff(agent_event_tx, &bundle).await;
```
- Add the helper:
```rust
/// Push a single kickoff task into the agent loop so a freshly specialized
/// agent starts working its role. The task is purpose-derived, NOT a
/// hard-coded routine — the swarm decides procedure at runtime (design D6).
async fn inject_role_kickoff(
    agent_event_tx: &Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::Sender<AgentEvent>>>>,
    bundle: &AgentSpecializationBundle,
) {
    let guard = agent_event_tx.lock().await;
    let Some(tx) = guard.as_ref() else {
        // No loop wired (e.g. specialist with no orchestrator loop, or tests
        // exercising only the persist path). Nothing to kick off.
        return;
    };
    let rc = &bundle.role_context;
    let handoffs = if rc.handoff_targets.is_empty() {
        String::new()
    } else {
        format!(" Hand off to: {}.", rc.handoff_targets.join(", "))
    };
    let content = format!(
        "You are now specialized as the '{}' role ({}) for SwarmKit flight {}. \
         Purpose: {}. Use ONLY your granted tools to do this role's work now.{}",
        rc.role_id, rc.role_type, rc.flight_id, bundle.persona.purpose, handoffs
    );
    // See Step 3a for the AgentEvent shape (reply channel handling).
    let _ = tx.send(/* AgentEvent::IncomingMessage { .. } */).await;
}
```

- [ ] **Step 3a: Build the `AgentEvent::IncomingMessage` (verified shape)**

Verified against `crates/arkavo-server/src/server/agent_event.rs`:
- `CorrelationId(pub uuid::Uuid)` — wraps a `Uuid`, NOT a String.
- `AgentEvent::IncomingMessage { sender: String, content: String, task_id: uuid::Uuid, correlation_id: CorrelationId, reply: oneshot::Sender<CycleReceipt> }` — `task_id` is a `Uuid` and `reply` is required.

`role_context.flight_id` is a String holding a UUID; parse it (fall back to a fresh UUID if it isn't canonical, since flight_id is orchestrator-supplied). There is no external caller awaiting the kickoff reply, so create a throwaway oneshot and drop the receiver:
```rust
    use crate::server::agent_event::CorrelationId;
    let task_id = rc.flight_id.parse::<uuid::Uuid>().unwrap_or_else(|_| uuid::Uuid::new_v4());
    let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
    let event = AgentEvent::IncomingMessage {
        sender: "swarmkit-orchestrator".to_string(),
        content,
        task_id,
        correlation_id: CorrelationId(uuid::Uuid::new_v4()),
        reply: reply_tx,
    };
    let _ = tx.send(event).await;
```
> The dropped `_reply_rx` means the loop's `CycleReceipt` send (agent_loop.rs:252-253) is a harmless no-op for this message — the loop already tolerates a closed reply channel (`let _ = sender.send(...)`).

`mod.rs`: in `agent_specialize` dispatch (mod.rs:1017-1031), add the new argument:
```rust
            &self.agent_event_tx,
```
(after `self.bundle_decryptor.as_ref()`, before `request`). `self.agent_event_tx` exists (mod.rs A2aRpcImpl field).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p arkavo-server --lib specialize_injects_role_kickoff_task`
Expected: PASS.
Run: `cargo test -p arkavo-server --lib handle_specialize` (the 4 pre-existing tests, updated to pass a `None` event-tx)
Expected: PASS.

- [ ] **Step 5: Build + clippy**

Run: `cargo build -p arkavo-server` (clean)
Run: `cargo clippy -p arkavo-server -- -D warnings` (clean)

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-server/src/server/handlers/specialization.rs \
        crates/arkavo-server/src/server/mod.rs
git commit -m "Kick off role execution on specialize via the agent-loop channel

After persona + grants + role context are applied, the specialize handler
injects one purpose-derived kickoff task into the agent loop through the
existing agent_event_tx channel. The task names the role and points the
agent at its granted tools; procedure stays emergent (design D6). No-op
when no loop is wired (specialists / tests).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task 5: Negative security test (end-to-end-ish: reviewer denied `github_pr_create`)

**Files:**
- Test: new `crates/arkavo-server/tests/least_privilege_specialization.rs` (integration test) OR inline in `conductor_tool_loop.rs` — see Step 1.

**Interfaces:**
- Consumes: `ToolRegistry::retain_granted`, `tool_call_permitted`, and the reviewer grant set from the github-ops-kit manifest (`git_diff`, `git_log`, `gh_pr_review`, `github_ci_status`).

- [ ] **Step 1: Choose the test seam**

A true end-to-end run needs a loaded model (network/GPU), which the Global Constraints forbid in tests. The security property is fully expressed by two checkable invariants without a model:
  1. **Registry boundary:** a reviewer-filtered registry does not contain `github_pr_create`.
  2. **Call-time boundary:** `tool_call_permitted("github_pr_create", Some(reviewer_set))` is `false`.
Implement Step 2 as the negative test. (The live-model E2E denial is asserted in WS-E's end-to-end run.)

- [ ] **Step 2: Write the negative test**

Add an integration test `crates/arkavo-server/tests/least_privilege_specialization.rs`:

```rust
//! WS-C negative security test: a reviewer-role agent cannot reach an
//! ungranted write tool (github_pr_create), at the registry boundary.

use std::collections::HashSet;
use std::sync::Arc;

#[tokio::test]
async fn reviewer_registry_excludes_ungranted_write_tool() {
    use arkavo_memory::MemoryStorage;
    use arkavo_mcp_tools::ToolRegistry;
    let storage = Arc::new(MemoryStorage::new_test().await.expect("storage"));
    let mut reg = ToolRegistry::new(storage);
    // Reviewer grants from examples/github-ops-kit (pr_reviewer role).
    let reviewer: HashSet<String> =
        ["git_diff", "git_log", "gh_pr_review", "github_ci_status"]
            .iter().map(|s| s.to_string()).collect();
    reg.retain_granted(&reviewer);

    assert!(reg.get("gh_pr_review").is_some(), "reviewer keeps its review tool");
    assert!(reg.get("git_diff").is_some());
    // The security assertion: the maintainer-only write tool is absent.
    assert!(reg.get("github_pr_create").is_none(), "reviewer must not see github_pr_create");
}
```

> `arkavo-server` dev-deps must include `arkavo-memory` and a tokio runtime with the `macros` + `rt` features for `#[tokio::test]` (both already present — the crate's lib tests use `#[tokio::test]`). If `arkavo-memory` is not a dev-dep of `arkavo-server`, add it under `[dev-dependencies]` and commit `Cargo.lock`.

> If `tool_call_permitted` is private (it is — `fn`, not `pub`), assert the boundary only at the registry layer here (public API). The call-time gate is unit-tested in Task 3 Step 1 inside the crate. That split keeps the public integration test clean while the in-crate test covers the private predicate.

- [ ] **Step 3: Run the negative test**

Run: `cargo test -p arkavo-server --test least_privilege_specialization`
Expected: PASS (1 test). `github_pr_create` is denied at the registry boundary; the call-time gate denial is proven by `ungranted_tool_call_is_denied` (Task 3).

- [ ] **Step 4: Full crate test + clippy + fmt**

Run: `cargo test -p arkavo-server` (PASS — lib + this integration test)
Run: `cargo clippy -p arkavo-server --tests -- -D warnings` (clean)
Run: `cargo fmt -- --check` (clean)

- [ ] **Step 5: Commit**

```bash
git add crates/arkavo-server/tests/least_privilege_specialization.rs
git commit -m "Add negative security test: reviewer role denied github_pr_create

A pr_reviewer-grant-filtered registry (git_diff/git_log/gh_pr_review/
github_ci_status from github-ops-kit) excludes the maintainer-only
github_pr_create write tool. Pairs with the in-crate call-time denial test
to assert least-privilege at both enforcement layers (design D9).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

## Self-Review

**Spec coverage (WS-C scope, from `swarmkit-pr-review-design.md` §WS-C + D9 + Missing items 3 & 4):**
- "Persist `persona.mcp_tools` in `AgentMetadata` on specialize (handler drops grants today)" → Task 2: `granted_tools` field + `granted_tool_names()` + persist in `apply_bundle_to_metadata`. ✓
- "Grant-filter the agent's `ToolRegistry` — only granted tools (agent_loop.rs ~118-143 builds ALL today)" → Task 1 (`retain_granted`) + Task 3 (`derive_filtered_registry`, memoized re-derivation in the tick branch, `active_registry`). ✓
- "Call-time enforcement — reject an ungranted tool before execution (conductor_tool_loop.rs)" → Task 3: `tool_call_permitted` gate at the top of `execute_tool_calls`, closing the fall-through-to-`mcp_registry.call_tool` hole. ✓
- "Post-specialize role execution — run the role's task through the conductor loop (today continuous-advisory)" → Task 4: kickoff injected via existing `agent_event_tx`; loop's tick cadence runs it with the filtered registry. ✓
- "Negative security test: reviewer attempting `github_pr_create` is denied" → Task 5 (registry boundary, public) + Task 3 (call-time, in-crate). ✓

**Grant-format correctness (load-bearing):** the manifest's `McpToolGrant.tools` are bare registry keys (`git_diff`, `gh_pr_review`, `github_pr_create` — verified against `registry.rs::register_all` and `github-ops-kit.swarmkit.yaml`). The handler's existing `activated` vec builds `server:tool` strings for the RPC *response only*; `granted_tool_names()` deliberately drops `server` and matches bare names. Filtering by `server:tool` would silently filter everything out — explicitly avoided and tested.

**Type/signature consistency:** the `Option<&HashSet<String>>` granted param is threaded `agent_loop` → `execute_with_conductor_and_learning` (after `cached_registry`, before iroh) → `run_tool_loop` / `run_tool_loop_parallel` → `execute_tool_calls`, identical type at every hop. `AgentLoopConfig.agent_metadata` reuses the exact `Arc<RwLock<AgentMetadata>>` already on `A2aRpcImpl`. `handle_agent_specialize`'s new `agent_event_tx` matches the existing `A2aRpcImpl.agent_event_tx` field type exactly.

**No-conflict with WS-D (the controller's sequencing concern):** WS-C's `specialization.rs` edits are confined to (a) `apply_bundle_to_metadata` (one new line + `granted_tool_names` helper), (b) one `inject_role_kickoff` call after `role_specialization.set`, (c) two appended params on `handle_agent_specialize`/`handle_inner`. WS-C does **not** touch the decode/decrypt prologue (lines 88-152) where WS-D inserts the iroh `fetch_bytes`. Both can land in either order with at most a trivial signature-line merge.

**Flagged decisions (DO NOT treat as settled — see report):**
- D-C1: filtered-registry re-derivation strategy (memoized rebuild in the loop) vs. handler mutating a shared registry handle.
- D-C2: post-specialize role execution = single kickoff message into the existing event channel vs. a dedicated role-runner.
- D-C3: enforcement is two-layer (filtered registry + call-time gate); the call-time gate returns a "Denied" tool result rather than aborting the whole loop.

**Placeholder scan:** No TBD steps. The two "confirm the exact shape" notes (Step 3a `CorrelationId`/`reply` field; Task 1 `MemoryStorage::default`) are named, checkable verifications against real types, not placeholders.
