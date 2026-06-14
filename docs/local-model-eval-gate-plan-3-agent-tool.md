# Eval as an Agent-Loop MCP Tool — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Depends on PR #625 (Part 1, `arkavo-eval` engine) already on the branch.**

**Goal:** Make the local-model eval a `run_eval` MCP tool the existing agent loop can call (alongside the GitHub MCP tools) so the Arkavo Edge swarm member monitors PRs and evaluates models from within `run_agent_loop` — no separate daemon. Lands in PR #625.

**Architecture:** `arkavo-eval` exports `register_tools(registry, Arc<EvalState>)` (the codebase tool pattern). `RunEvalTool` runs the real `LlamaOperator` + a persistent baseline store + the ONNX semantic embedder and returns a verdict. `arkavo-server`'s `agent_loop.rs` registers it into the conductor's `ToolRegistry`; the LLM composes it with the existing `github_pr_list` / `gh_pr_review` tools per the agent's `purpose`.

**Tech Stack:** Rust, `async-trait`, `arkavo_mcp_tools::server::Tool`/`ToolSchema`/`ToolRegistry`, `arkavo-llm` (llama.cpp), `arkavo-memory` (ONNX embedder), `serde_json`.

---

## File structure

`arkavo-eval`:
- `src/operator_llama.rs` — real `LlamaOperator` (already written during the E2E; this plan **commits** it).
- `src/embedder.rs` — `MemoryEmbedder` (ONNX, feature `embeddings`) + `CharEmbedder` fallback.
- `src/baseline_file.rs` — `FileBaselineStore` (persistent, JSON-on-disk, keyed by `(commit,model)`).
- `src/tool.rs` — `EvalState`, `RunEvalTool`, `register_tools()` (feature `mcp-tool`).
- `src/verdict.rs` — **modify**: make `cosine` a free fn and `assess` take `&dyn Embedder` (so `EvalState` can hold `Arc<dyn Embedder>`).
- `src/lib.rs` — module decls + re-export `register_tools`, `EvalState`.
- `Cargo.toml` — deps/features (`embeddings`, `llama-cpp`, `mcp-tool`).

`arkavo-server`:
- `src/server/agent_loop.rs` — `AgentLoopConfig.eval_state` + register call (feature-gated).
- `src/server/a2a_server.rs` — populate `eval_state` in `start_orchestrator_loop`.
- `Cargo.toml` — `arkavo-eval` dep (feature-gated `eval-tool`).

---

## Phase A — Commit the real Operator + make the embedder pluggable

### Task 1: Commit the LlamaOperator + register the module/feature

**Files:**
- Create: `crates/arkavo-eval/src/operator_llama.rs` (already exists in the working tree from the E2E)
- Modify: `crates/arkavo-eval/src/lib.rs`, `crates/arkavo-eval/Cargo.toml`

- [ ] **Step 1: Confirm the operator + feature exist** (they were added during the E2E run)

Run: `sed -n '1,12p' crates/arkavo-eval/src/operator_llama.rs && grep -nE 'llama-cpp|arkavo-llm' crates/arkavo-eval/Cargo.toml`
Expected: `LlamaOperator` present; `arkavo-llm` optional dep + `llama-cpp = ["dep:arkavo-llm", "arkavo-llm/llama-cpp"]` feature; `pub mod operator_llama` under `#[cfg(feature="llama-cpp")]` in lib.rs.

If any is missing, add per the E2E (`operator_llama.rs` uses `arkavo_llm::{LlamaCppProvider, Message, Provider, SamplingConfig}` and reads `ParsedToolCall.tool_name`).

- [ ] **Step 2: Build to confirm**

Run: `cargo build -p arkavo-eval --features llama-cpp 2>&1 | tail -3`
Expected: Finished.

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/operator_llama.rs crates/arkavo-eval/src/lib.rs crates/arkavo-eval/Cargo.toml
git commit -m "$(printf 'arkavo-eval: real llama.cpp Operator\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

### Task 2: Make `cosine` a free fn and `assess` take `&dyn Embedder`

**Files:**
- Modify: `crates/arkavo-eval/src/verdict.rs`
- Modify: `crates/arkavo-eval/src/lib.rs` (the `assess` call in `run_eval`)

Why: `EvalState` must hold `Arc<dyn Embedder>`. The current `assess<E: Embedder>` calls `E::cosine` (a `Self: Sized` method) which a `dyn Embedder` can't satisfy. Move `cosine` out of the trait.

- [ ] **Step 1: In `verdict.rs`, replace the `Embedder` trait + `assess` signature**

Change the trait to have ONLY `embed`, add a free `cosine`, and make `assess` take `&dyn Embedder`:

```rust
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError>;
}

/// Cosine similarity over the overlapping prefix of two vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

pub async fn assess(
    embed: &dyn Embedder,
    outputs: &[PromptOutput],
    baseline: &Baseline,
    min_similarity: f64,
    min_tok_s_ratio: f64,
) -> Result<TypedStatus, VerdictError> {
    // ... unchanged body, but replace `E::cosine(&va, &vb)` with `cosine(&va, &vb)`
}
```

Update the body's `sim_sum += E::cosine(&va, &vb) as f64;` → `sim_sum += cosine(&va, &vb) as f64;`. Update the inline tests: `assess(&FakeEmbedder, ...)` still works (`&FakeEmbedder` coerces to `&dyn Embedder`). Remove the `cosine` test's `E::cosine` references → call the free `cosine`.

- [ ] **Step 2: In `lib.rs` `run_eval`, the `assess(embed, ...)` call**

`run_eval<O,B,E>(... embed: &E ...)` calls `assess(embed, ...)`. `&E` coerces to `&dyn Embedder` automatically — no change needed. Build to confirm.

- [ ] **Step 3: Run tests**

Run: `cargo nextest run -p arkavo-eval verdict && cargo nextest run -p arkavo-eval --test pipeline`
Expected: all PASS (the refactor is behavior-preserving).

Run: `cargo clippy -p arkavo-eval -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/arkavo-eval/src/verdict.rs crates/arkavo-eval/src/lib.rs
git commit -m "$(printf 'arkavo-eval: cosine as free fn; assess over &dyn Embedder\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

### Task 3: `embedder.rs` — MemoryEmbedder (ONNX) + CharEmbedder fallback

**Files:**
- Create: `crates/arkavo-eval/src/embedder.rs`
- Modify: `crates/arkavo-eval/src/lib.rs`

- [ ] **Step 1: Write the module**

```rust
//! Embedder implementations. `MemoryEmbedder` is the production semantic
//! embedder (arkavo-memory's bundled offline ONNX model); `CharEmbedder` is a
//! deterministic fallback used when the ONNX model files are not present.

use crate::verdict::{Embedder, VerdictError};
use async_trait::async_trait;

/// Deterministic char-frequency embedder (no external model).
pub struct CharEmbedder;

#[async_trait]
impl Embedder for CharEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        let mut v = vec![0.0f32; 27];
        for c in text.to_lowercase().chars() {
            if c.is_ascii_lowercase() {
                v[(c as u8 - b'a') as usize] += 1.0;
            } else {
                v[26] += 1.0;
            }
        }
        Ok(v)
    }
}

#[cfg(feature = "embeddings")]
pub struct MemoryEmbedder {
    inner: arkavo_memory::EmbeddingService,
}

#[cfg(feature = "embeddings")]
impl MemoryEmbedder {
    pub fn new() -> Self {
        Self { inner: arkavo_memory::EmbeddingService::new() }
    }
    /// True if the bundled ONNX model files are loadable in this process.
    pub async fn available(&self) -> bool {
        self.inner.ensure_model_available().await.is_ok()
    }
}

#[cfg(feature = "embeddings")]
impl Default for MemoryEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl Embedder for MemoryEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        self.inner
            .generate_embedding(text)
            .await
            .map_err(|e| VerdictError::Embedding(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn char_embedder_is_deterministic() {
        let a = CharEmbedder.embed("paris").await.unwrap();
        let b = CharEmbedder.embed("paris").await.unwrap();
        assert_eq!(a, b);
        assert_eq!(crate::verdict::cosine(&a, &b), 1.0);
    }
}
```

> Confirm `arkavo_memory::EmbeddingService` exposes `new()`, `ensure_model_available()`, and `generate_embedding(&str)` (it does — `crates/arkavo-memory/src/embeddings.rs`). Confirm `EmbeddingService` is re-exported from `arkavo_memory` crate root; if not, add `pub use embeddings::EmbeddingService;` to `crates/arkavo-memory/src/lib.rs`.

- [ ] **Step 2: Wire the module + re-export**

In `crates/arkavo-eval/src/lib.rs` add:

```rust
pub mod embedder;
```

- [ ] **Step 3: Test + commit**

Run: `cargo nextest run -p arkavo-eval embedder && cargo clippy -p arkavo-eval --features embeddings -- -D warnings`
Expected: PASS, clean.

```bash
git add crates/arkavo-eval/src/embedder.rs crates/arkavo-eval/src/lib.rs
git commit -m "$(printf 'arkavo-eval: MemoryEmbedder (ONNX) + CharEmbedder fallback\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Phase B — Persistent baseline store

### Task 4: `baseline_file.rs` — `FileBaselineStore`

**Files:**
- Create: `crates/arkavo-eval/src/baseline_file.rs`
- Modify: `crates/arkavo-eval/src/lib.rs`

Baselines must survive across eval calls (the agent is long-running; evals happen over time). A JSON-on-disk store keyed by `(commit, model)`.

- [ ] **Step 1: Write the module + test**

```rust
//! Filesystem-backed BaselineStore: persists each baseline as JSON under a
//! directory, keyed by a sanitized `(commit, model)`. Survives restarts.

use crate::baseline::{BaselineError, BaselinePointer, BaselineStore};
use crate::digest::b3_hex;
use crate::verdict::Baseline;
use async_trait::async_trait;
use std::path::PathBuf;

pub struct FileBaselineStore {
    dir: PathBuf,
}

impl FileBaselineStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).ok();
        Self { dir }
    }

    fn key(commit: &str, model: &str) -> String {
        let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect::<String>();
        format!("{}__{}.json", safe(commit), safe(model))
    }

    fn path(&self, commit: &str, model: &str) -> PathBuf {
        self.dir.join(Self::key(commit, model))
    }
}

#[async_trait]
impl BaselineStore for FileBaselineStore {
    async fn fetch(&self, commit: &str, model: &str) -> Result<Option<Baseline>, BaselineError> {
        match std::fs::read(self.path(commit, model)) {
            Ok(bytes) => {
                let b: Baseline = serde_json::from_slice(&bytes).map_err(|e| BaselineError::Backend(e.to_string()))?;
                Ok(Some(b))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BaselineError::Backend(e.to_string())),
        }
    }

    async fn publish(&self, commit: &str, model: &str, baseline: &Baseline) -> Result<BaselinePointer, BaselineError> {
        let bytes = serde_json::to_vec_pretty(baseline).map_err(|e| BaselineError::Backend(e.to_string()))?;
        std::fs::write(self.path(commit, model), &bytes).map_err(|e| BaselineError::Backend(e.to_string()))?;
        Ok(BaselinePointer {
            commit: commit.into(),
            model: model.into(),
            b3_digest: b3_hex(&bytes),
            ticket: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::BaselineOutput;

    #[tokio::test]
    async fn persists_across_instances() {
        let dir = std::env::temp_dir().join(format!("arkavo-eval-fbs-{}", std::process::id()));
        let b = Baseline { outputs: vec![BaselineOutput { id: "p1".into(), text: "paris".into() }], tok_s: 10.0 };
        {
            let store = FileBaselineStore::new(&dir);
            assert!(store.fetch("c1", "m").await.unwrap().is_none());
            store.publish("c1", "m", &b).await.unwrap();
        }
        // New instance, same dir — baseline survives.
        let store2 = FileBaselineStore::new(&dir);
        assert_eq!(store2.fetch("c1", "m").await.unwrap().unwrap(), b);
        std::fs::remove_dir_all(dir).ok();
    }
}
```

- [ ] **Step 2: Wire + test + commit**

In `lib.rs`: `pub mod baseline_file;`

Run: `cargo nextest run -p arkavo-eval baseline_file && cargo clippy -p arkavo-eval -- -D warnings`
Expected: PASS, clean.

```bash
git add crates/arkavo-eval/src/baseline_file.rs crates/arkavo-eval/src/lib.rs
git commit -m "$(printf 'arkavo-eval: filesystem-backed BaselineStore\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Phase C — The `run_eval` MCP tool

### Task 5: `tool.rs` — `EvalState`, `RunEvalTool`, `register_tools`

**Files:**
- Create: `crates/arkavo-eval/src/tool.rs`
- Modify: `crates/arkavo-eval/src/lib.rs`, `crates/arkavo-eval/Cargo.toml`

- [ ] **Step 1: Add deps + the `mcp-tool` feature**

In `crates/arkavo-eval/Cargo.toml`:

```toml
[dependencies]
# ... existing ...
arkavo-mcp-tools = { path = "../arkavo-mcp-tools", optional = true }

[features]
# ... existing embeddings/llama-cpp ...
# The run_eval MCP tool needs the real operator, the semantic embedder, and the tool trait.
mcp-tool = ["dep:arkavo-mcp-tools", "llama-cpp", "embeddings"]
```

> Confirm the `Tool`/`ToolSchema`/`ToolRegistry`/`Result`/`ToolError` exact paths from `crates/arkavo-mcp-tools/src/{server.rs,registry.rs,lib.rs}` before writing — the code below uses `arkavo_mcp_tools::server::Tool`, `arkavo_mcp_tools::ToolSchema`, `arkavo_mcp_tools::ToolRegistry`, `arkavo_mcp_tools::Result`, `arkavo_mcp_tools::ToolError::Execution`.

- [ ] **Step 2: Write `tool.rs`**

```rust
//! `run_eval` MCP tool: runs the local-model eval suite on a model and returns
//! a pass/regression verdict vs the recorded baseline. Registered into the
//! agent loop's tool registry so the conductor can call it.

use crate::baseline::BaselineStore;
use crate::contract::{EvalPrompt, ExecutionProfile, ModelSpec, PromptMessage};
use crate::operator::Operator;
use crate::operator_llama::LlamaOperator;
use crate::plan::EvalPlan;
use crate::verdict::{assess, Baseline, BaselineOutput, Embedder};
use arkavo_mcp_tools::server::Tool;
use arkavo_mcp_tools::{ToolError, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Shared state for the eval tools.
pub struct EvalState {
    pub embedder: Arc<dyn Embedder>,
    pub baselines: Arc<dyn BaselineStore>,
    /// Default capability prompt-set used when the caller doesn't supply one.
    pub prompts: Vec<EvalPrompt>,
    /// Resolve a model name to a local GGUF path (e.g. from the HF cache).
    pub resolve_model: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

impl EvalState {
    /// The default capability prompt-set.
    pub fn default_prompts() -> Vec<EvalPrompt> {
        let user = |id: &str, content: &str| EvalPrompt {
            id: id.into(),
            messages: vec![PromptMessage { role: "user".into(), content: content.into() }],
            tools: None,
        };
        vec![
            user("capital", "What is the capital of France? Answer in one word."),
            user("arithmetic", "A car travels 140 km in 2 hours. Average speed in km/h? Answer with the number and unit."),
            user("instruct", "List exactly three primary colors, comma-separated, nothing else."),
        ]
    }
}

pub struct RunEvalTool {
    schema: ToolSchema,
    state: Arc<EvalState>,
}

impl RunEvalTool {
    pub fn new(state: Arc<EvalState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "run_eval".to_string(),
                aliases: None,
                description: "Run the local-model evaluation suite on a model and return a pass/regression verdict versus the recorded baseline. Use to gate PRs that change model behavior. Args: model (name), baseline_ref (label the baseline is keyed under, e.g. a git ref; default 'main'), update_baseline (record this run as the new baseline).".to_string(),
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
impl Tool for RunEvalTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let model = args.get("model").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Execution("run_eval: missing 'model'".into()))?;
        let baseline_ref = args.get("baseline_ref").and_then(|v| v.as_str()).unwrap_or("main");
        let update_baseline = args.get("update_baseline").and_then(|v| v.as_bool()).unwrap_or(false);

        let path = (self.state.resolve_model)(model)
            .ok_or_else(|| ToolError::Execution(format!("run_eval: model '{model}' not resident on this swarm member")))?;

        let plan = EvalPlan {
            model: ModelSpec { name: model.to_string(), quant: "Q4_K_M".into(), weight_digest: "b3:agent".into() },
            prompts: self.state.prompts.clone(),
            exec: ExecutionProfile { seed: 0, temperature: 0.0, threads: None, ctx: None, max_tokens: 48 },
            baseline_commit: Some(baseline_ref.to_string()),
        };

        let run = LlamaOperator::new(model.to_string(), path)
            .run(&plan)
            .await
            .map_err(|e| ToolError::Execution(format!("run_eval: {e}")))?;

        let existing = self.state.baselines.fetch(baseline_ref, model).await
            .map_err(|e| ToolError::Execution(format!("run_eval baseline fetch: {e}")))?;

        let mean_tok_s = if run.outputs.is_empty() { 0.0 } else {
            run.outputs.iter().map(|o| o.tok_s).sum::<f64>() / run.outputs.len() as f64
        };
        let outputs_json: Vec<Value> = run.outputs.iter()
            .map(|o| json!({ "id": o.id, "text": o.text, "tok_s": o.tok_s }))
            .collect();

        let (status, recorded) = match (&existing, update_baseline) {
            (None, _) | (Some(_), true) => {
                // Bootstrap (or overwrite) the baseline from this run.
                let new_baseline = Baseline {
                    outputs: run.outputs.iter().map(|o| BaselineOutput { id: o.id.clone(), text: o.text.clone() }).collect(),
                    tok_s: mean_tok_s,
                };
                self.state.baselines.publish(baseline_ref, model, &new_baseline).await
                    .map_err(|e| ToolError::Execution(format!("run_eval baseline publish: {e}")))?;
                ("baseline_bootstrapped".to_string(), true)
            }
            (Some(base), false) => {
                let verdict = assess(self.state.embedder.as_ref(), &run.outputs, base, 0.87, 0.95).await
                    .map_err(|e| ToolError::Execution(format!("run_eval verdict: {e}")))?;
                (verdict_kind(&verdict), false)
            }
        };

        Ok(json!({
            "model": model,
            "baseline_ref": baseline_ref,
            "status": status,
            "baseline_recorded": recorded,
            "mean_tok_s": mean_tok_s,
            "outputs": outputs_json,
        }))
    }
}

fn verdict_kind(status: &crate::status::TypedStatus) -> String {
    serde_json::to_value(status).ok()
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".into())
}

/// Register the eval tools into a ToolRegistry (codebase pattern).
pub fn register_tools(registry: &mut arkavo_mcp_tools::ToolRegistry, state: Arc<EvalState>) {
    registry.register("run_eval", Box::new(RunEvalTool::new(state)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operator::{OperatorError, PromptOutput, RunOutput};
    use crate::verdict::VerdictError;

    struct FakeOp(String);
    #[async_trait]
    impl Operator for FakeOp {
        async fn run(&self, plan: &EvalPlan) -> Result<RunOutput, OperatorError> {
            Ok(RunOutput { outputs: plan.prompts.iter().map(|p| PromptOutput { id: p.id.clone(), text: self.0.clone(), tok_s: 10.0 }).collect() })
        }
    }
    struct FakeEmb;
    #[async_trait]
    impl Embedder for FakeEmb {
        async fn embed(&self, t: &str) -> Result<Vec<f32>, VerdictError> {
            let mut v = vec![0.0f32; 27];
            for c in t.to_lowercase().chars() { if c.is_ascii_lowercase() { v[(c as u8 - b'a') as usize] += 1.0; } else { v[26] += 1.0; } }
            Ok(v)
        }
    }

    // This test exercises the bootstrap->assess logic by calling the same
    // building blocks RunEvalTool::execute uses, with a fake operator (no model).
    #[tokio::test]
    async fn bootstrap_then_pass_via_building_blocks() {
        use crate::baseline_file::FileBaselineStore;
        let dir = std::env::temp_dir().join(format!("arkavo-eval-tool-{}", std::process::id()));
        let store = FileBaselineStore::new(&dir);
        let plan = EvalPlan {
            model: ModelSpec { name: "m".into(), quant: "q".into(), weight_digest: "b3:0".into() },
            prompts: EvalState::default_prompts(),
            exec: ExecutionProfile { seed: 0, temperature: 0.0, threads: None, ctx: None, max_tokens: 8 },
            baseline_commit: Some("main".into()),
        };
        // bootstrap
        let r1 = FakeOp("paris".into()).run(&plan).await.unwrap();
        let base = Baseline { outputs: r1.outputs.iter().map(|o| BaselineOutput { id: o.id.clone(), text: o.text.clone() }).collect(), tok_s: 10.0 };
        store.publish("main", "m", &base).await.unwrap();
        // assess (same outputs) -> passed
        let r2 = FakeOp("paris".into()).run(&plan).await.unwrap();
        let fetched = store.fetch("main", "m").await.unwrap().unwrap();
        let status = assess(&FakeEmb, &r2.outputs, &fetched, 0.87, 0.95).await.unwrap();
        assert_eq!(verdict_kind(&status), "passed");
        std::fs::remove_dir_all(dir).ok();
    }
}
```

- [ ] **Step 3: Wire the module (feature-gated) + re-exports**

In `crates/arkavo-eval/src/lib.rs`:

```rust
#[cfg(feature = "mcp-tool")]
pub mod tool;
#[cfg(feature = "mcp-tool")]
pub use tool::{register_tools, EvalState};
```

- [ ] **Step 4: Build + test + clippy**

Run: `cargo build -p arkavo-eval --features mcp-tool 2>&1 | tail -5`
Expected: Finished (pulls llama-cpp + embeddings + arkavo-mcp-tools).

Run: `cargo nextest run -p arkavo-eval --features mcp-tool tool::`
Expected: `bootstrap_then_pass_via_building_blocks` PASS.

Run: `cargo clippy -p arkavo-eval --features mcp-tool -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/arkavo-eval/src/tool.rs crates/arkavo-eval/src/lib.rs crates/arkavo-eval/Cargo.toml
git commit -m "$(printf 'arkavo-eval: run_eval MCP tool + EvalState + register_tools\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Phase D — Wire into the agent loop

### Task 6: Add `eval_state` to `AgentLoopConfig` + register in the loop

**Files:**
- Modify: `crates/arkavo-server/src/server/agent_loop.rs`
- Modify: `crates/arkavo-server/Cargo.toml`

- [ ] **Step 1: Add the dep (feature-gated) to arkavo-server**

In `crates/arkavo-server/Cargo.toml`:

```toml
[dependencies]
# ... existing ...
arkavo-eval = { path = "../arkavo-eval", optional = true }

[features]
# Add to the feature set the agent binary uses (mirror how llama-cpp is plumbed).
eval-tool = ["dep:arkavo-eval", "arkavo-eval/mcp-tool"]
```

> Check `arkavo-server`'s existing `[features]` and how `llama-cpp` reaches it; add `eval-tool` to the same default chain the `arkavo` binary enables (so the agent gets the tool). Read `crates/arkavo/Cargo.toml` default features + `crates/arkavo-cli/Cargo.toml` to thread `eval-tool` through (it should imply the llama path).

- [ ] **Step 2: Add the field to `AgentLoopConfig`** (around `agent_loop.rs:15-38`)

```rust
    #[cfg(feature = "eval-tool")]
    pub eval_state: Option<std::sync::Arc<arkavo_eval::EvalState>>,
```

- [ ] **Step 3: Register the tool where the registry is built** (around `agent_loop.rs:131`, right after `arkavo_mcp_mesh::register_tools(...)`)

```rust
        #[cfg(feature = "eval-tool")]
        if let Some(ref eval_state) = config.eval_state {
            arkavo_eval::register_tools(&mut registry, eval_state.clone());
            tracing::info!("registered run_eval tool for the agent loop");
        }
```

- [ ] **Step 4: Build**

Run: `cargo build -p arkavo-server --features eval-tool 2>&1 | tail -5`
Expected: Finished. (Fix any `AgentLoopConfig` construction sites that now need the new field — see Task 7.)

- [ ] **Step 5: Commit**

```bash
git add crates/arkavo-server/src/server/agent_loop.rs crates/arkavo-server/Cargo.toml
git commit -m "$(printf 'arkavo-server: register run_eval tool in the agent loop\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

### Task 7: Populate `eval_state` in `start_orchestrator_loop`

**Files:**
- Modify: `crates/arkavo-server/src/server/a2a_server.rs`

- [ ] **Step 1: Build the `EvalState`** in `start_orchestrator_loop` (where `AgentLoopConfig` is constructed, ~`a2a_server.rs:1325-1399`)

Add, before constructing the config:

```rust
    #[cfg(feature = "eval-tool")]
    let eval_state = {
        // Embedder: prefer the ONNX semantic model, fall back to char-frequency.
        let mem = arkavo_eval::embedder::MemoryEmbedder::new();
        let embedder: std::sync::Arc<dyn arkavo_eval::verdict::Embedder> = if mem.available().await {
            std::sync::Arc::new(mem)
        } else {
            tracing::warn!("ONNX embedder unavailable; run_eval will use the char-frequency fallback");
            std::sync::Arc::new(arkavo_eval::embedder::CharEmbedder)
        };
        let baseline_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("arkavo/eval-baselines");
        Some(std::sync::Arc::new(arkavo_eval::EvalState {
            embedder,
            baselines: std::sync::Arc::new(arkavo_eval::baseline_file::FileBaselineStore::new(baseline_dir)),
            prompts: arkavo_eval::EvalState::default_prompts(),
            resolve_model: std::sync::Arc::new(resolve_gguf_from_hf_cache),
        }))
    };
```

Add a module-level helper in `a2a_server.rs` (or a small `fn`):

```rust
/// Resolve a model name to a local GGUF path from the HuggingFace cache.
/// Mirrors how the gemma tests locate weights.
#[cfg(feature = "eval-tool")]
fn resolve_gguf_from_hf_cache(model: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    // Try a couple of common unsloth/ggml repo layouts for the given model name.
    let candidates = [
        format!("{home}/.cache/huggingface/hub/models--unsloth--{model}-GGUF"),
        format!("{home}/.cache/huggingface/hub/models--ggml-org--{model}-GGUF"),
    ];
    for base in candidates {
        let snaps = std::fs::read_dir(format!("{base}/snapshots")).ok();
        if let Some(snaps) = snaps {
            for snap in snaps.flatten() {
                if let Ok(files) = std::fs::read_dir(snap.path()) {
                    for f in files.flatten() {
                        let p = f.path();
                        if p.extension().is_some_and(|e| e == "gguf") {
                            return Some(p.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }
    None
}
```

> Confirm `dirs` is a dependency of `arkavo-server` (it's a workspace dep); add it if needed.

- [ ] **Step 2: Add `eval_state` to the `AgentLoopConfig { ... }` construction**

```rust
            #[cfg(feature = "eval-tool")]
            eval_state,
```

- [ ] **Step 3: Fix any other `AgentLoopConfig` construction sites**

Run: `grep -rn 'AgentLoopConfig {' crates/arkavo-server/src`
For each, add the `#[cfg(feature = "eval-tool")] eval_state: None,` field (or the real one) so they compile under the feature.

- [ ] **Step 4: Build (with and without the feature)**

Run: `cargo build -p arkavo-server 2>&1 | tail -3` (no feature — must still compile)
Run: `cargo build -p arkavo-server --features eval-tool 2>&1 | tail -3`
Expected: both Finished.

- [ ] **Step 5: clippy + commit**

Run: `cargo clippy -p arkavo-server --features eval-tool -- -D warnings`

```bash
git add crates/arkavo-server/src/server/a2a_server.rs crates/arkavo-server/Cargo.toml
git commit -m "$(printf 'arkavo-server: build EvalState (ONNX embedder + file baselines) for the agent loop\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

### Task 8: Thread `eval-tool` into the `arkavo` binary's default features

**Files:**
- Modify: `crates/arkavo/Cargo.toml`, `crates/arkavo-cli/Cargo.toml`

- [ ] **Step 1: Add the feature through the chain**

The agent binary is `arkavo` → `arkavo-cli` → `arkavo-server`. Add an `eval-tool` feature to each that forwards to the next, and include it in the `arkavo` default (next to `llama-cpp`):

- `crates/arkavo-server/Cargo.toml`: done (Task 6).
- `crates/arkavo-cli/Cargo.toml`: `eval-tool = ["arkavo-server/eval-tool"]` (and add to the cli's default/llama feature set).
- `crates/arkavo/Cargo.toml`: `eval-tool = ["arkavo-cli/eval-tool"]`, and add `"eval-tool"` to `default = [...]`.

> Read each crate's `[features]` first and mirror exactly how `llama-cpp` is forwarded (same dependency edges). Keep `minimal`/`windows-default` WITHOUT `eval-tool`.

- [ ] **Step 2: Build the binary (debug)**

Run: `cargo build -p arkavo 2>&1 | tail -3`
Expected: Finished (the default now includes `eval-tool`).

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo/Cargo.toml crates/arkavo-cli/Cargo.toml Cargo.lock
git commit -m "$(printf 'arkavo: enable run_eval tool in the default agent build\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

---

## Phase E — Verify the agent can call it

### Task 9: Live verification (`#[ignore]` + manual)

**Files:**
- Create: `crates/arkavo-eval/tests/run_eval_tool.rs`

- [ ] **Step 1: `#[ignore]` integration test that calls the tool with a real model**

```rust
#![cfg(feature = "mcp-tool")]

use arkavo_eval::tool::{EvalState, RunEvalTool};
use arkavo_mcp_tools::server::Tool;
use serde_json::json;
use std::sync::Arc;

fn find_model() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let base = format!("{home}/.cache/huggingface/hub/models--unsloth--gemma-3-4b-it-GGUF");
    for snap in std::fs::read_dir(format!("{base}/snapshots")).ok()?.flatten() {
        for f in std::fs::read_dir(snap.path()).ok()?.flatten() {
            let p = f.path();
            if p.extension().is_some_and(|e| e == "gguf") {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

#[tokio::test]
#[ignore = "requires a local gemma-3-4b model"]
async fn run_eval_tool_bootstrap_then_pass() {
    let Some(path) = find_model() else { eprintln!("skip: no model"); return; };
    let dir = std::env::temp_dir().join(format!("arkavo-eval-rt-{}", std::process::id()));
    let resolve = {
        let path = path.clone();
        Arc::new(move |_m: &str| Some(path.clone())) as Arc<dyn Fn(&str) -> Option<String> + Send + Sync>
    };
    let state = Arc::new(EvalState {
        embedder: Arc::new(arkavo_eval::embedder::CharEmbedder),
        baselines: Arc::new(arkavo_eval::baseline_file::FileBaselineStore::new(&dir)),
        prompts: EvalState::default_prompts(),
        resolve_model: resolve,
    });
    let tool = RunEvalTool::new(state);

    let boot = tool.execute(json!({ "model": "gemma-3-4b" })).await.unwrap();
    assert_eq!(boot["status"], "baseline_bootstrapped");

    let pass = tool.execute(json!({ "model": "gemma-3-4b" })).await.unwrap();
    assert_eq!(pass["status"], "passed");

    std::fs::remove_dir_all(dir).ok();
}
```

- [ ] **Step 2: Run it locally (with the model present)**

Run: `cargo nextest run -p arkavo-eval --features mcp-tool --run-ignored all run_eval_tool_bootstrap`
Expected: PASS (bootstrap then passed).

- [ ] **Step 3: Manual end-to-end via the agent**

Document in the PR: run the agent with a `purpose` that references the tool, confirm `registered run_eval tool` in logs, and that asking the agent to "run_eval on gemma-3-4b" invokes the tool. Example AGENTS.md purpose snippet:

```
purpose: |
  Monitor open PRs in arkavo-org/arkavo-edge. For PRs that change model-behavior
  code, call run_eval on the affected model and post the verdict as a PR comment
  using gh_pr_review (event=COMMENT). Use github_pr_list to find PRs.
```

- [ ] **Step 4: Commit**

```bash
git add crates/arkavo-eval/tests/run_eval_tool.rs
git commit -m "$(printf 'arkavo-eval: ignored live test for the run_eval tool\n\nCo-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>')"
```

### Task 10: Pre-push checklist

- [ ] **Step 1:** `cargo fmt -- --check`
- [ ] **Step 2:** `cargo build -q` (full workspace — now possible with the vendor pin fixed)
- [ ] **Step 3:** `cargo clippy --workspace -- -D warnings` (or scope to touched crates if unrelated noise)
- [ ] **Step 4:** Bump `[workspace.package] version` (feature completion) and commit `Cargo.lock`.
- [ ] **Step 5:** Security tests per CLAUDE.md (`cargo test -p arkavo-protocol --test security_vulnerabilities`, `cargo test -p arkavo-cli mock_provider`, the DLP/PII scripts).
- [ ] **Step 6:** Push; confirm CI green on PR #625.

---

## Self-review notes (author)

- **Spec coverage:** `run_eval` MCP tool (Task 5) ✓; agent-loop registration (Tasks 6–8) ✓; persistence (Task 4) ✓; ONNX embedder + fallback (Tasks 3, 7) ✓; commits the real Operator (Task 1) ✓; addresses the E2E findings — `baseline_present` is handled by the tool's contract (not a refuse precondition) and the ONNX embedder replaces the char placeholder; tok/s remains best-effort (see below).
- **Deliberately deferred (flagged):** Check Runs (need the GitHub App — the agent posts comments via `gh_pr_review` for now); the `tok_s` timing capture is an `arkavo-llm` fix (inference_timing came back 0 in the E2E) — until then the tok/s gate is inert (baseline tok_s 0 ⇒ ratio defaults to pass), which is safe but not enforcing; eligibility/discovery is left to the agent composing `github_pr_list` (per its purpose) rather than a hardcoded scan.
- **Reliability note:** the gate is LLM-invoked (per the chosen design); for a hard *required* check, a deterministic `eval_tick` step could be added later. `run_eval` itself is deterministic and self-contained when called.
