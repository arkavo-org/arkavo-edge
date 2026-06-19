# SwarmKit PR-review — WS-A (Fold-ins) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Vendor the `arkavo-eval` pipeline (#625) and the `github-ops-kit` manifest (#626) onto `feature/swarmkit-pr-review`, exposing the eval gate as the `test_eval` tool inside `arkavo-mcp-tools` (the testing MCP).

**Architecture:** `arkavo-eval` lands as a **pipeline-only** library (no MCP-tool module, to avoid a dependency cycle). `arkavo-mcp-tools` gains an optional `eval` feature that depends on `arkavo-eval` and registers a `test_eval` tool next to `test_run`. `arkavo-server` constructs the runtime `EvalState` (embedder + file baseline store + HF-cache model resolver) and registers the tool when its `eval` feature is on. The `github-ops-kit` manifest + validation test are vendored and adjusted to grant `test_eval` and `github_pr_watch`.

**Tech Stack:** Rust, `cargo nextest`/`cargo test`, `async-trait`, `serde_json`, llama.cpp (feature-gated), `arkavo-eval`, `arkavo-mcp-tools`, `arkavo-swarmkit`.

## Global Constraints

- No `--release` builds during development; use debug (`cargo build -q`).
- No clippy warnings: `cargo clippy -- -D warnings`. `#[allow(dead_code)]` forbidden.
- No OpenSSL anywhere — `rustls` only.
- Implementation files (excluding `#[cfg(test)]`) stay under 400 lines; split by responsibility.
- llama.cpp (C++) must be **feature-gated off by default** so slim/musl/Windows builds stay C++-free.
- No Conventional Commits (no `feat:`/`fix:` prefixes). Commit `Cargo.lock` whenever `Cargo.toml` changes.
- Every bug fix gets a regression test. Tests: `cargo nextest run` preferred.
- Commit message trailer (per repo): end commits with the `Co-Authored-By` / `Claude-Session` lines used on this branch.

---

## File Structure

- `crates/arkavo-eval/` — vendored pipeline lib (drop `src/tool.rs`, `tests/run_eval_tool.rs`; drop `mcp-tool` feature + `arkavo-mcp-tools` dep).
- `crates/arkavo-mcp-tools/src/eval.rs` — **new**: `TestEvalTool` + `EvalState` re-export + `register_eval_tool`. The only new implementation file.
- `crates/arkavo-mcp-tools/src/{lib.rs,registry.rs,Cargo.toml}` — wire the `eval` feature + module.
- `crates/arkavo-server/src/server/{a2a_server.rs,agent_loop.rs,Cargo.toml}` — construct `EvalState`, register `test_eval` (relocated from #625's `eval-tool` wiring).
- `Cargo.toml` (workspace) — add `crates/arkavo-eval` to `members`.
- `examples/github-ops-kit/github-ops-kit.swarmkit.yaml` — vendored, grants adjusted (`test_eval`, `github_pr_watch`).
- `crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs` — vendored, assertions adjusted.

---

### Task A1: Vendor `arkavo-eval` as a pipeline-only crate

**Files:**
- Create (vendored): `crates/arkavo-eval/**` (from `origin/feature/local-model-eval-gate`)
- Delete: `crates/arkavo-eval/src/tool.rs`, `crates/arkavo-eval/tests/run_eval_tool.rs`
- Modify: `crates/arkavo-eval/Cargo.toml`, `crates/arkavo-eval/src/lib.rs`, `Cargo.toml` (workspace `members`)
- Test: `crates/arkavo-eval/tests/pipeline.rs` (vendored as-is)

**Interfaces:**
- Produces: crate `arkavo-eval` with public `run_eval(...)`, `RunOutcome { status, published }`, modules `contract`, `operator`, `operator_llama` (feat `llama-cpp`), `embedder` (`LexicalEmbedder`), `verdict` (`assess`, `Baseline`, `BaselineOutput`, `Embedder`), `status` (`TypedStatus` with `check_conclusion()` + `summary()`), `baseline` (`BaselineStore`, `BaselinePointer`), `baseline_file::FileBaselineStore`, `plan::EvalPlan`. Features: `default=[]`, `embeddings`, `tdf-baselines`, `llama-cpp`. **No `mcp-tool` feature, no `arkavo-mcp-tools` dependency.**

- [ ] **Step 1: Vendor the crate from the #625 branch**

```bash
cd /Users/arkavo/Projects/arkavo/arkavo-edge
git fetch origin feature/local-model-eval-gate --quiet
git checkout origin/feature/local-model-eval-gate -- crates/arkavo-eval
```

- [ ] **Step 2: Remove the MCP-tool module + its test (relocating to arkavo-mcp-tools)**

```bash
git rm -f crates/arkavo-eval/src/tool.rs crates/arkavo-eval/tests/run_eval_tool.rs
```

- [ ] **Step 3: Drop the `mcp-tool` feature + `arkavo-mcp-tools` dep from `crates/arkavo-eval/Cargo.toml`**

Remove the `arkavo-mcp-tools = { ... optional = true }` dependency line, and remove the `mcp-tool` feature line so `[features]` reads exactly:

```toml
[features]
default = []
embeddings = ["dep:arkavo-memory", "arkavo-memory/embeddings"]
# Trusted baseline store: TDF-encrypted, content-addressed over a blob transport.
tdf-baselines = ["dep:arkavo-tdf"]
# Real Operator backed by arkavo-llm's llama.cpp provider.
llama-cpp = ["dep:arkavo-llm", "arkavo-llm/llama-cpp"]
```

- [ ] **Step 4: Drop the `tool` module from `crates/arkavo-eval/src/lib.rs`**

Delete these four lines:

```rust
#[cfg(feature = "mcp-tool")]
pub mod tool;
#[cfg(feature = "mcp-tool")]
pub use tool::{register_tools, EvalState};
```

- [ ] **Step 5: Register the crate in the workspace**

In the workspace `Cargo.toml`, add to the `members` array (alphabetical-ish, near the other eval/mcp crates):

```toml
    "crates/arkavo-eval",
```

- [ ] **Step 6: Build + run the pipeline tests**

Run: `cargo test -p arkavo-eval`
Expected: PASS (the `pipeline.rs` integration tests + inline unit tests; `operator_llama`/`tool` excluded — no llama.cpp pulled).

- [ ] **Step 7: Clippy + commit**

Run: `cargo clippy -p arkavo-eval -- -D warnings`
Expected: clean.

```bash
git add crates/arkavo-eval Cargo.toml Cargo.lock
git commit -m "Vendor arkavo-eval as a pipeline-only crate (from #625)

Drops the in-crate run_eval MCP tool and its arkavo-mcp-tools dep; the tool
relocates to arkavo-mcp-tools (avoids a dependency cycle). Pipeline + verdict
logic land unchanged.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task A2: Add the `test_eval` tool to `arkavo-mcp-tools`

**Files:**
- Create: `crates/arkavo-mcp-tools/src/eval.rs`
- Modify: `crates/arkavo-mcp-tools/Cargo.toml` (add `eval` feature + optional `arkavo-eval` dep), `crates/arkavo-mcp-tools/src/lib.rs` (gate the module + re-export)
- Test: inline `#[cfg(test)]` in `eval.rs`

**Interfaces:**
- Consumes (A1): `arkavo_eval::{EvalState, baseline::BaselineStore, contract::{EvalPrompt, ExecutionProfile, ModelSpec, PromptMessage}, operator::Operator, operator_llama::LlamaOperator, plan::EvalPlan, verdict::{assess, Baseline, BaselineOutput, Embedder}, status::TypedStatus}`.
- Produces: `arkavo_mcp_tools::eval::{TestEvalTool, register_eval_tool}` and re-export `arkavo_mcp_tools::EvalState`. Tool name: `test_eval`. `register_eval_tool(registry: &mut ToolRegistry, state: Arc<EvalState>)`.

- [ ] **Step 1: Add the `eval` feature + dep to `crates/arkavo-mcp-tools/Cargo.toml`**

Under `[dependencies]` add:

```toml
# Local-model eval gate (test_eval). Pulls llama.cpp via arkavo-eval; off by default
# so slim/musl/Windows stay C++-free.
arkavo-eval = { path = "../arkavo-eval", optional = true, features = ["llama-cpp"] }
```

Under `[features]` add:

```toml
eval = ["dep:arkavo-eval"]
```

(Leave `default = ["iroh", "code-tools"]` unchanged — `eval` is opt-in.)

- [ ] **Step 2: Write `crates/arkavo-mcp-tools/src/eval.rs` (the relocated tool, named `test_eval`)**

```rust
//! `test_eval` MCP tool: runs the local-model eval suite on a model and returns
//! a pass/regression verdict vs the recorded baseline. Lives in the testing MCP
//! (alongside `test_run`); the eval pipeline itself is the `arkavo-eval` crate.

use crate::server::Tool;
use crate::{ToolError, ToolSchema};
use arkavo_eval::baseline::BaselineStore;
use arkavo_eval::contract::{EvalPrompt, ExecutionProfile, ModelSpec, PromptMessage};
use arkavo_eval::operator::Operator;
use arkavo_eval::operator_llama::LlamaOperator;
use arkavo_eval::plan::EvalPlan;
use arkavo_eval::status::TypedStatus;
use arkavo_eval::verdict::{assess, Baseline, BaselineOutput, Embedder};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Resolves a model name to a local GGUF path.
pub type ModelResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Shared state for the eval tool.
pub struct EvalState {
    pub embedder: Arc<dyn Embedder>,
    pub baselines: Arc<dyn BaselineStore>,
    /// Default capability prompt-set used when the caller doesn't supply one.
    pub prompts: Vec<EvalPrompt>,
    /// Resolve a model name to a local GGUF path (e.g. from the HF cache).
    pub resolve_model: ModelResolver,
}

impl EvalState {
    /// The default capability prompt-set.
    pub fn default_prompts() -> Vec<EvalPrompt> {
        let user = |id: &str, content: &str| EvalPrompt {
            id: id.into(),
            messages: vec![PromptMessage {
                role: "user".into(),
                content: content.into(),
            }],
            tools: None,
        };
        vec![
            user("capital_au", "What is the capital of Australia? Answer with one word."),
            user("arithmetic", "What is 17 multiplied by 23? Answer with just the number."),
            user("reverse", "Reverse the letters of the word 'algorithm'. Answer with only the reversed string, nothing else."),
            user("symbol", "What is the chemical symbol for gold? Answer with just the symbol."),
            user("primes", "List the first five prime numbers, comma-separated, nothing else."),
        ]
    }
}

pub struct TestEvalTool {
    schema: ToolSchema,
    state: Arc<EvalState>,
}

impl TestEvalTool {
    pub fn new(state: Arc<EvalState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "test_eval".to_string(),
                aliases: None,
                description: "Run the local-model evaluation suite on a model and return a pass/regression verdict versus the recorded baseline. Use to gate PRs that change model behavior. The result includes `check_conclusion` (GitHub Check Run conclusion: success/failure/neutral/action_required, or null to skip posting) and `summary` — after running, post these to the PR (a GitHub Check Run on the head SHA, or a PR comment) using your configured GitHub MCP tool. Args: model (name), baseline_ref (label the baseline is keyed under, e.g. a git ref; default 'main'), update_baseline (record this run as the new baseline).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "model": { "type": "string", "description": "Model name to evaluate (resolved to a local GGUF)." },
                        "baseline_ref": { "type": "string", "description": "Key the baseline is stored under. Default 'main'." },
                        "update_baseline": { "type": "boolean", "description": "If true, record this run as the new baseline. Default false." }
                    },
                    "required": ["model"]
                }),
            },
            state,
        }
    }
}

#[async_trait]
impl Tool for TestEvalTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Execution("test_eval: missing 'model'".into()))?;
        let baseline_ref = args
            .get("baseline_ref")
            .and_then(|v| v.as_str())
            .unwrap_or("main");
        let update_baseline = args
            .get("update_baseline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = (self.state.resolve_model)(model).ok_or_else(|| {
            ToolError::Execution(format!(
                "test_eval: model '{model}' not resident on this swarm member"
            ))
        })?;

        let plan = EvalPlan {
            model: ModelSpec {
                name: model.to_string(),
                quant: "Q4_K_M".into(),
                weight_digest: "b3:agent".into(),
            },
            prompts: self.state.prompts.clone(),
            exec: ExecutionProfile {
                seed: 0,
                temperature: 0.0,
                threads: None,
                ctx: None,
                max_tokens: 48,
            },
            baseline_commit: Some(baseline_ref.to_string()),
        };

        let run = LlamaOperator::new(model.to_string(), path)
            .run(&plan)
            .await
            .map_err(|e| ToolError::Execution(format!("test_eval: {e}")))?;

        let existing = self
            .state
            .baselines
            .fetch(baseline_ref, model)
            .await
            .map_err(|e| ToolError::Execution(format!("test_eval baseline fetch: {e}")))?;

        let mean_tok_s = if run.outputs.is_empty() {
            0.0
        } else {
            run.outputs.iter().map(|o| o.tok_s).sum::<f64>() / run.outputs.len() as f64
        };
        let outputs_json: Vec<Value> = run
            .outputs
            .iter()
            .map(|o| json!({ "id": o.id, "text": o.text, "tok_s": o.tok_s }))
            .collect();

        let (typed_status, recorded) = match (&existing, update_baseline) {
            (None, _) | (Some(_), true) => {
                let new_baseline = Baseline {
                    outputs: run
                        .outputs
                        .iter()
                        .map(|o| BaselineOutput {
                            id: o.id.clone(),
                            text: o.text.clone(),
                        })
                        .collect(),
                    tok_s: mean_tok_s,
                };
                self.state
                    .baselines
                    .publish(baseline_ref, model, &new_baseline)
                    .await
                    .map_err(|e| ToolError::Execution(format!("test_eval baseline publish: {e}")))?;
                (TypedStatus::BaselineBootstrapped, true)
            }
            (Some(base), false) => {
                let verdict = assess(self.state.embedder.as_ref(), &run.outputs, base, 0.87, 0.95)
                    .await
                    .map_err(|e| ToolError::Execution(format!("test_eval verdict: {e}")))?;
                (verdict, false)
            }
        };

        Ok(eval_result(
            model,
            baseline_ref,
            &typed_status,
            recorded,
            mean_tok_s,
            outputs_json,
        ))
    }
}

fn verdict_kind(status: &TypedStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".into())
}

fn eval_result(
    model: &str,
    baseline_ref: &str,
    status: &TypedStatus,
    recorded: bool,
    mean_tok_s: f64,
    outputs: Vec<Value>,
) -> Value {
    json!({
        "model": model,
        "baseline_ref": baseline_ref,
        "status": verdict_kind(status),
        "check_conclusion": status.check_conclusion(),
        "summary": status.summary(),
        "baseline_recorded": recorded,
        "mean_tok_s": mean_tok_s,
        "outputs": outputs,
    })
}

/// Register the eval tool into a ToolRegistry (codebase `register_*` pattern).
pub fn register_eval_tool(registry: &mut crate::ToolRegistry, state: Arc<EvalState>) {
    registry.register("test_eval", Box::new(TestEvalTool::new(state)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_named_test_eval_and_categorizes_as_testing() {
        let state = Arc::new(EvalState {
            embedder: Arc::new(arkavo_eval::embedder::LexicalEmbedder::new()),
            baselines: Arc::new(arkavo_eval::baseline::MemBaselineStore::default()),
            prompts: EvalState::default_prompts(),
            resolve_model: Arc::new(|_| None),
        });
        let tool = TestEvalTool::new(state);
        assert_eq!(tool.schema().name, "test_eval");
        assert!(tool.schema().name.starts_with("test_"));
    }
}
```

> Note: confirm `arkavo_eval::baseline::MemBaselineStore` exists (it is the in-memory `BaselineStore` from #625). If the type name differs, use the in-memory store actually exported by `arkavo_eval::baseline`.

- [ ] **Step 3: Gate the module + re-export in `crates/arkavo-mcp-tools/src/lib.rs`**

Add near the other `pub mod` lines:

```rust
#[cfg(feature = "eval")]
pub mod eval;
#[cfg(feature = "eval")]
pub use eval::{register_eval_tool, EvalState};
```

- [ ] **Step 4: Run the tool unit test (eval feature on)**

Run: `cargo test -p arkavo-mcp-tools --features eval eval::tests`
Expected: PASS (`schema_is_named_test_eval_and_categorizes_as_testing`).

- [ ] **Step 5: Verify the default build stays C++-free**

Run: `cargo build -p arkavo-mcp-tools` (no `--features eval`)
Expected: builds clean; `arkavo-eval`/llama.cpp NOT compiled.

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy -p arkavo-mcp-tools --features eval -- -D warnings`
Expected: clean.

```bash
git add crates/arkavo-mcp-tools/src/eval.rs crates/arkavo-mcp-tools/src/lib.rs crates/arkavo-mcp-tools/Cargo.toml Cargo.lock
git commit -m "Add test_eval tool to arkavo-mcp-tools behind the eval feature

Relocates #625's run_eval into the testing MCP as test_eval (Testing category),
wrapping the arkavo-eval pipeline. Feature-gated so llama.cpp stays out of
default/slim/Windows builds.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task A3: Construct `EvalState` + register `test_eval` in `arkavo-server`

**Files:**
- Modify: `crates/arkavo-server/Cargo.toml` (replace #625's `eval-tool` feature with an `eval` feature → `arkavo-mcp-tools/eval`), `crates/arkavo-server/src/server/a2a_server.rs` (construct `EvalState` from `arkavo_mcp_tools`), `crates/arkavo-server/src/server/agent_loop.rs` (register via `arkavo_mcp_tools::register_eval_tool`)
- Test: inline build/feature check (no new runtime test — covered by A2 + WS-C)

**Interfaces:**
- Consumes (A2): `arkavo_mcp_tools::{EvalState, register_eval_tool}`, `arkavo_eval::{embedder::LexicalEmbedder, baseline_file::FileBaselineStore}`.
- Produces: an agent whose tool registry includes `test_eval` when built `--features eval`.

- [ ] **Step 1: Replace the `eval-tool` feature in `crates/arkavo-server/Cargo.toml`**

Change the feature so it routes through `arkavo-mcp-tools` instead of a direct `arkavo-eval` MCP wiring:

```toml
# Local-model eval gate, surfaced as the arkavo-mcp-tools `test_eval` tool.
eval = ["arkavo-mcp-tools/eval", "dep:arkavo-eval"]
```

Keep the `arkavo-eval` dep (used only to construct `EvalState`'s embedder + baseline store), marked `optional = true`:

```toml
arkavo-eval = { path = "../arkavo-eval", optional = true, features = ["llama-cpp"] }
```

Remove any prior `eval-tool = [...]` feature line.

- [ ] **Step 2: Construct `EvalState` via `arkavo_mcp_tools` in `a2a_server.rs`**

Replace #625's `arkavo_eval::EvalState { .. }` construction block (around the existing `eval_state` let-binding) with the `arkavo_mcp_tools` type. Under `#[cfg(feature = "eval")]`:

```rust
let eval_state = {
    let embedder: std::sync::Arc<dyn arkavo_eval::verdict::Embedder> =
        std::sync::Arc::new(arkavo_eval::embedder::LexicalEmbedder::new());
    let baseline_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("arkavo/eval-baselines");
    Some(std::sync::Arc::new(arkavo_mcp_tools::EvalState {
        embedder,
        baselines: std::sync::Arc::new(
            arkavo_eval::baseline_file::FileBaselineStore::new(baseline_dir),
        ),
        prompts: arkavo_mcp_tools::EvalState::default_prompts(),
        resolve_model: std::sync::Arc::new(resolve_gguf_from_hf_cache),
    }))
};
```

Keep #625's `resolve_gguf_from_hf_cache` helper and pass `eval_state` into the agent-loop config exactly as before (the field type changes from `arkavo_eval::EvalState` to `arkavo_mcp_tools::EvalState`).

- [ ] **Step 3: Register the tool via `arkavo_mcp_tools` in `agent_loop.rs`**

Where #625 called `arkavo_eval::register_tools(registry, state)`, call the relocated registrar (under `#[cfg(feature = "eval")]`):

```rust
if let Some(state) = config.eval_state.clone() {
    arkavo_mcp_tools::register_eval_tool(&mut registry, state);
}
```

Change the `AgentLoopConfig.eval_state` field type to `Option<std::sync::Arc<arkavo_mcp_tools::EvalState>>`.

- [ ] **Step 4: Confirm `arkavo-cli` has NO `arkavo-eval` dependency**

Run: `grep -n "arkavo-eval" crates/arkavo-cli/Cargo.toml`
Expected: no output. If a line exists (added by #625), remove it.

- [ ] **Step 5: Build with the eval feature**

Run: `cargo build -p arkavo-server --features eval`
Expected: builds; `test_eval` is wired into the agent registry.

- [ ] **Step 6: Build without it (slim path)**

Run: `cargo build -p arkavo-server`
Expected: builds clean, no llama.cpp.

- [ ] **Step 7: Clippy + commit**

Run: `cargo clippy -p arkavo-server --features eval -- -D warnings`
Expected: clean.

```bash
git add crates/arkavo-server/Cargo.toml crates/arkavo-server/src/server/a2a_server.rs crates/arkavo-server/src/server/agent_loop.rs crates/arkavo-cli/Cargo.toml Cargo.lock
git commit -m "Wire test_eval into the server agent loop via arkavo-mcp-tools

Constructs EvalState (lexical embedder + file baseline store + HF-cache model
resolver) and registers the relocated test_eval tool under the eval feature.
Drops #625's bespoke eval-tool wiring and the stray arkavo-cli arkavo-eval dep.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task A4: Vendor the `github-ops-kit` manifest + validation test (#626), adjust grants

**Files:**
- Create (vendored): `examples/github-ops-kit/github-ops-kit.swarmkit.yaml`, `crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs` (from `origin/feature/github-ops-kit`)
- Modify: the vendored manifest (grants) + the vendored test (assertions)
- Test: `crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs`

**Interfaces:**
- Produces: a validated `github-ops-kit` manifest whose `dispatcher` grants `github_pr_watch` and whose `pr_test_runner` grants `test_eval` (instead of `run_eval`).

- [ ] **Step 1: Vendor the manifest + test from the #626 branch**

```bash
cd /Users/arkavo/Projects/arkavo/arkavo-edge
git fetch origin feature/github-ops-kit --quiet
git checkout origin/feature/github-ops-kit -- examples/github-ops-kit crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs
```

- [ ] **Step 2: Rename the gate tool grant `run_eval` → `test_eval` in the manifest**

In `examples/github-ops-kit/github-ops-kit.swarmkit.yaml`, under the `pr_test_runner` role's `mcp_tools`, change the granted tool name from `run_eval` to `test_eval` (server label stays `arkavo-mcp-tools`):

```yaml
      - server: "arkavo-mcp-tools"
        tools: ["test_eval"]
        auth: none
```

- [ ] **Step 3: Add the `github_pr_watch` grant to the `dispatcher` role**

In the `dispatcher` role's `mcp_tools`, add the PR-watch grant (the tool itself ships in WS-B; the manifest validation only checks structure):

```yaml
      - server: "arkavo-mcp-tools"
        tools: ["github_pr_watch"]
        auth: delegated
```

- [ ] **Step 4: Update the validation test assertions**

In `crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs`, change any assertion referencing `run_eval` to `test_eval`, and add an assertion that the `dispatcher` role grants `github_pr_watch`. Example (adapt to the test's existing helper for finding a role's granted tools):

```rust
// pr_test_runner gates via the relocated eval tool
assert!(role_tools(&manifest, "pr_test_runner").contains(&"test_eval".to_string()));
// dispatcher monitors PRs via the poll tool
assert!(role_tools(&manifest, "dispatcher").contains(&"github_pr_watch".to_string()));
```

- [ ] **Step 5: Run the validation test**

Run: `cargo test -p arkavo-swarmkit --test github_ops_kit_validates`
Expected: PASS (parses, cross-block `validate()`, kit-id round-trip, least-privilege boundaries, the two new grant assertions).

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy -p arkavo-swarmkit --tests -- -D warnings`
Expected: clean.

```bash
git add examples/github-ops-kit crates/arkavo-swarmkit/tests/github_ops_kit_validates.rs
git commit -m "Vendor github-ops-kit manifest + validation test (from #626)

Adjusts grants for this branch: pr_test_runner gates via test_eval; dispatcher
gets the github_pr_watch poll grant (tool lands in WS-B).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

## Self-Review

**Spec coverage (WS-A scope):**
- "Fold #625 as the eval pipeline lib; drop tool.rs/mcp-tool, server eval-tool wiring, CLI dep" → A1 (vendor pipeline-only), A3 (relocate wiring, drop CLI dep). ✓
- "`test_eval` in arkavo-mcp-tools (Testing category), feature-gated for llama.cpp" → A2. ✓
- "Eval pipeline stays a lib arkavo-mcp-tools depends on" → A1 + A2 dep direction (no cycle). ✓
- "Fold #626 manifest + validation test; reference test_eval; add github_pr_watch grant to dispatcher" → A4. ✓

**Placeholder scan:** No TBD/"add error handling"/vague steps; all code/commands concrete. The one explicit verify-this note (`MemBaselineStore` type name) is a named, checkable assumption, not a placeholder.

**Type consistency:** `EvalState` fields (`embedder`/`baselines`/`prompts`/`resolve_model`) identical across A2 (definition) and A3 (construction). `register_eval_tool(&mut ToolRegistry, Arc<EvalState>)` matches its A3 call site. Tool name `test_eval` consistent across A2 (schema), A4 (manifest grant + test). `TypedStatus::{check_conclusion,summary,BaselineBootstrapped}` used as in #625.

**Open risks carried to planning of later WS:** `github_pr_watch` tool does not yet exist (WS-B); the A4 manifest grant is structural-only and validated as such — acceptable because manifest `validate()` checks shape, not tool existence.
