# Local-Model Eval Gate — Implementation Plan (Part 1: Core + GitHub Gate)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `arkavo-eval` pipeline (Planner→Critic-gate→Operator→Critic-verdict→Scribe→Historian as trait-bounded modules) and wire it to GitHub Check Runs, producing a working, deterministic PR gate using fake/in-memory backends — no local model, TDF, or iroh required yet.

**Architecture:** A new `arkavo-eval` crate holds the five roles as small modules behind traits (`Operator`, `BaselineStore`, `Embedder`). `run_eval()` orchestrates them and returns a `TypedStatus`. The pre-flight gate is a real `torg_core` boolean AND-circuit over the contract's preconditions. The verdict is cosine similarity (via an `Embedder` trait) plus a tok/s ratio. `arkavo-github` gains `create_check_run`/`update_check_run`. A one-shot `arkavo eval run` CLI proves the end-to-end loop.

**Tech Stack:** Rust, tokio, `async-trait`, `serde`/`serde_json`, `blake3`, `torg-core`, `arkavo-memory::EmbeddingService` (behind a trait), `reqwest` (rustls), `cargo nextest`.

**Part 2** (separate plan) adds the real llama.cpp Operator, TDF+iroh baseline distribution, and the org-wide daemon.

---

## File structure (Part 1)

New crate `crates/arkavo-eval/`:

- `Cargo.toml` — deps + features (`embeddings`, later `llama-cpp`, `tdf-iroh`).
- `src/lib.rs` — re-exports + `run_eval()` orchestration + `RunOutcome`.
- `src/digest.rs` — `b3:<hex>` helpers.
- `src/status.rs` — `TypedStatus` + Check Run conclusion mapping.
- `src/contract.rs` — `EvalContract` and nested types.
- `src/plan.rs` — Planner: `EvalPlan` + `plan()`.
- `src/gate.rs` — Critic pre-flight: `Preconditions`, `evaluate_gate()` (torg circuit).
- `src/verdict.rs` — Critic post-flight: `Embedder` trait, `assess()`.
- `src/operator.rs` — `Operator` trait, `RunOutput`, `PromptOutput`, `FakeOperator` (test).
- `src/baseline.rs` — `BaselineStore` trait, `Baseline`, `BaselinePointer`, `MemBaselineStore` (test).

Modified:

- `crates/arkavo-github/src/operations.rs` — add `create_check_run` (returns id) + `update_check_run`.
- `Cargo.toml` (workspace) — add `crates/arkavo-eval` member.
- `crates/arkavo-cli/src/lib.rs` + `src/commands/eval.rs` — `arkavo eval run` one-shot subcommand.
- `crates/arkavo-cli/src/commands/mod.rs` — declare `eval` module.

---

## Phase 0 — Scaffold the crate

### Task 0: Create `arkavo-eval` crate

**Files:**
- Create: `crates/arkavo-eval/Cargo.toml`
- Create: `crates/arkavo-eval/src/lib.rs`
- Modify: `Cargo.toml` (workspace members list)

- [ ] **Step 1: Add the crate to the workspace members**

In `Cargo.toml`, find the `members = [` list and add this line alongside the other `crates/...` entries:

```toml
    "crates/arkavo-eval",
```

- [ ] **Step 2: Write `crates/arkavo-eval/Cargo.toml`**

```toml
[package]
name = "arkavo-eval"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
async-trait = { workspace = true }
thiserror = { workspace = true }
blake3 = { workspace = true }
torg-core = { workspace = true }
tracing = { workspace = true }
arkavo-memory = { path = "../arkavo-memory", optional = true }

[features]
default = []
# Wraps arkavo-memory::EmbeddingService as the real Embedder.
embeddings = ["dep:arkavo-memory", "arkavo-memory/embeddings"]

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

> If any of these are not already `[workspace.dependencies]`, use the version the rest of the repo uses (check another crate's `Cargo.toml`). `torg-core` is declared at workspace root (`Cargo.toml` line ~251) as `torg-core = { version = "0.2", features = ["serde"] }`; reference it via `torg-core = { workspace = true }` if a workspace entry exists, otherwise copy that exact line.

- [ ] **Step 3: Write a minimal `crates/arkavo-eval/src/lib.rs`**

```rust
//! Local-model evaluation pipeline: resolves an eval contract, gates on
//! preconditions, runs the model, and produces a typed regression verdict.

pub mod baseline;
pub mod contract;
pub mod digest;
pub mod gate;
pub mod operator;
pub mod plan;
pub mod status;
pub mod verdict;
```

- [ ] **Step 4: Verify it builds (empty modules will fail — that's expected next task)**

Run: `cargo build -p arkavo-eval 2>&1 | head -20`
Expected: FAIL with "file not found for module `baseline`" etc. (modules created in later tasks).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/arkavo-eval/Cargo.toml crates/arkavo-eval/src/lib.rs
git commit -m "Scaffold arkavo-eval crate"
```

---

## Phase 1 — Content addressing, status taxonomy, contract

### Task 1: `digest.rs` — `b3:<hex>` helpers

**Files:**
- Create: `crates/arkavo-eval/src/digest.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/arkavo-eval/src/digest.rs`:

```rust
//! BLAKE3 content addressing in the `b3:<64-hex>` form the eval contract uses.
//! (The existing `arkavo_swarmkit::canonical::content_hash` emits a different
//! `blake3:<base64url>` form and is left untouched.)

use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum DigestError {
    #[error("digest must start with 'b3:'")]
    MissingPrefix,
    #[error("digest hex must be 64 chars, got {0}")]
    BadLength(usize),
    #[error("invalid hex in digest")]
    BadHex,
}

/// Hash bytes and return `b3:<64 lowercase hex>`.
pub fn b3_hex(bytes: &[u8]) -> String {
    format!("b3:{}", blake3::hash(bytes).to_hex())
}

/// Parse a `b3:<hex>` string into 32 raw bytes.
pub fn parse_b3(s: &str) -> Result<[u8; 32], DigestError> {
    let hex = s.strip_prefix("b3:").ok_or(DigestError::MissingPrefix)?;
    if hex.len() != 64 {
        return Err(DigestError::BadLength(hex.len()));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16).ok_or(DigestError::BadHex)?;
        let lo = (chunk[1] as char).to_digit(16).ok_or(DigestError::BadHex)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Ok(out)
}

/// True if `bytes` hash to `expected` (a `b3:<hex>` string).
pub fn verify_b3(bytes: &[u8], expected: &str) -> bool {
    b3_hex(bytes) == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_verify() {
        let d = b3_hex(b"hello");
        assert!(d.starts_with("b3:"));
        assert_eq!(d.len(), 3 + 64);
        assert!(verify_b3(b"hello", &d));
        assert!(!verify_b3(b"world", &d));
        let raw = parse_b3(&d).unwrap();
        assert_eq!(raw, *blake3::hash(b"hello").as_bytes());
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert_eq!(parse_b3("xx:abc"), Err(DigestError::MissingPrefix));
        assert_eq!(parse_b3("b3:abc"), Err(DigestError::BadLength(3)));
        let bad = format!("b3:{}", "z".repeat(64));
        assert_eq!(parse_b3(&bad), Err(DigestError::BadHex));
    }
}
```

- [ ] **Step 2: Run the tests, verify they pass**

Run: `cargo nextest run -p arkavo-eval digest 2>&1 | tail -20` (or `cargo test -p arkavo-eval digest`)
Expected: PASS (2 tests). If the crate doesn't yet compile because other modules are empty, temporarily comment out the other `pub mod` lines in `lib.rs`, run, then restore them.

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/digest.rs
git commit -m "arkavo-eval: b3:<hex> content-addressing helpers"
```

### Task 2: `status.rs` — typed status taxonomy

**Files:**
- Create: `crates/arkavo-eval/src/status.rs`

- [ ] **Step 1: Write the module + tests**

```rust
//! Terminal eval status and its mapping onto GitHub Check Run conclusions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypedStatus {
    /// Acceptance met.
    Passed,
    /// A metric fell below its threshold.
    RegressionFailed {
        metric: String,
        value: f64,
        threshold: f64,
    },
    /// Pre-flight gate denied (digest mismatch, baseline absent+required, …).
    Refused { reason: String },
    /// First run for this model/prompt-set; nothing to compare against.
    BaselineBootstrapped,
    /// Infrastructure failure (model load, swarm error) — NOT a model regression.
    InfraError { stage: String },
    /// PR did not touch model paths; no check is posted.
    Skipped,
}

impl TypedStatus {
    /// GitHub Check Run `conclusion`, or `None` when no check should be posted.
    pub fn check_conclusion(&self) -> Option<&'static str> {
        match self {
            TypedStatus::Passed => Some("success"),
            TypedStatus::RegressionFailed { .. } => Some("failure"),
            TypedStatus::Refused { .. } => Some("action_required"),
            TypedStatus::BaselineBootstrapped => Some("neutral"),
            TypedStatus::InfraError { .. } => Some("failure"),
            TypedStatus::Skipped => None,
        }
    }

    /// One-line human summary for the check output title.
    pub fn summary(&self) -> String {
        match self {
            TypedStatus::Passed => "Eval passed".into(),
            TypedStatus::RegressionFailed { metric, value, threshold } => {
                format!("Regression: {metric} {value:.4} < {threshold:.4}")
            }
            TypedStatus::Refused { reason } => format!("Refused: {reason}"),
            TypedStatus::BaselineBootstrapped => "Baseline bootstrapped (neutral)".into(),
            TypedStatus::InfraError { stage } => format!("Infrastructure error at {stage}"),
            TypedStatus::Skipped => "Skipped".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conclusions_map_correctly() {
        assert_eq!(TypedStatus::Passed.check_conclusion(), Some("success"));
        assert_eq!(
            TypedStatus::RegressionFailed { metric: "similarity".into(), value: 0.5, threshold: 0.87 }
                .check_conclusion(),
            Some("failure")
        );
        assert_eq!(
            TypedStatus::Refused { reason: "x".into() }.check_conclusion(),
            Some("action_required")
        );
        assert_eq!(TypedStatus::BaselineBootstrapped.check_conclusion(), Some("neutral"));
        assert_eq!(
            TypedStatus::InfraError { stage: "operator".into() }.check_conclusion(),
            Some("failure")
        );
        assert_eq!(TypedStatus::Skipped.check_conclusion(), None);
    }

    #[test]
    fn infra_error_is_distinct_from_regression() {
        // Both map to "failure" but their summaries must be unambiguous.
        let infra = TypedStatus::InfraError { stage: "operator".into() };
        let reg = TypedStatus::RegressionFailed { metric: "similarity".into(), value: 0.1, threshold: 0.87 };
        assert!(infra.summary().contains("Infrastructure"));
        assert!(!reg.summary().contains("Infrastructure"));
    }
}
```

- [ ] **Step 2: Run tests, verify pass**

Run: `cargo nextest run -p arkavo-eval status`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/status.rs
git commit -m "arkavo-eval: typed status taxonomy + Check Run conclusion mapping"
```

### Task 3: `contract.rs` — the Eval Task Contract

**Files:**
- Create: `crates/arkavo-eval/src/contract.rs`

- [ ] **Step 1: Write the module + a serde round-trip test**

```rust
//! The Eval Task Contract: the single source of truth for what an eval runs.
//! Committed to the repo and content-addressed; references models/baselines by
//! `b3:<hex>` digest, never a mutable key.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalContract {
    pub contract_id: String,
    /// Always "model_eval" for this pipeline.
    pub task_kind: String,
    pub model: ModelSpec,
    pub baseline: BaselineRef,
    pub prompts: Vec<EvalPrompt>,
    pub acceptance: Acceptance,
    pub execution: ExecutionProfile,
    /// Names of required preconditions, e.g. ["weights_present","baseline_present"].
    pub preconditions: Vec<String>,
    /// torg circuit reference, e.g. "torg:eval-preflight-v1".
    pub policy_circuit: String,
    /// "refuse" is the only supported value in this slice.
    pub on_precondition_unmet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub name: String,
    pub quant: String,
    /// `b3:<hex>` of the GGUF weights.
    pub weight_digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineRef {
    /// "reference_outputs".
    pub kind: String,
    /// Git commit the baseline is anchored to (the lookup key). Optional on the
    /// very first run before any baseline exists.
    pub commit: Option<String>,
    /// Resolved `b3:<hex>` of the baseline artifact, if known.
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalPrompt {
    pub id: String,
    pub messages: Vec<PromptMessage>,
    /// Optional tool definitions (serde_json array) for tool-calling prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Acceptance {
    /// Minimum aggregate cosine similarity vs baseline (e.g. 0.87).
    pub min_similarity: f64,
    /// Minimum tok/s as a fraction of the baseline tok/s (e.g. 0.95).
    pub min_tok_s_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionProfile {
    pub seed: u32,
    pub temperature: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx: Option<u32>,
    pub max_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EvalContract {
        EvalContract {
            contract_id: "eval/gemma-4-12b/abc123".into(),
            task_kind: "model_eval".into(),
            model: ModelSpec {
                name: "gemma-4-12b".into(),
                quant: "Q4_K_M".into(),
                weight_digest: "b3:".to_string() + &"0".repeat(64),
            },
            baseline: BaselineRef { kind: "reference_outputs".into(), commit: Some("main0".into()), digest: None },
            prompts: vec![EvalPrompt {
                id: "capital".into(),
                messages: vec![PromptMessage { role: "user".into(), content: "Capital of France?".into() }],
                tools: None,
            }],
            acceptance: Acceptance { min_similarity: 0.87, min_tok_s_ratio: 0.95 },
            execution: ExecutionProfile { seed: 0, temperature: 0.0, threads: None, ctx: None, max_tokens: 64 },
            preconditions: vec!["weights_present".into(), "baseline_present".into()],
            policy_circuit: "torg:eval-preflight-v1".into(),
            on_precondition_unmet: "refuse".into(),
        }
    }

    #[test]
    fn json_round_trip() {
        let c = sample();
        let json = serde_json::to_string(&c).unwrap();
        let back: EvalContract = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
```

- [ ] **Step 2: Run tests, verify pass**

Run: `cargo nextest run -p arkavo-eval contract`
Expected: PASS (1 test).

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/contract.rs
git commit -m "arkavo-eval: Eval Task Contract types"
```

---

## Phase 2 — Pre-flight gate (TØR-G)

### Task 4: `gate.rs` — preconditions → boolean AND circuit → allow/deny

**Files:**
- Create: `crates/arkavo-eval/src/gate.rs`

- [ ] **Step 1: Write the module + tests**

```rust
//! Critic pre-flight gate. The contract's required preconditions are AND-ed
//! together as a real torg_core boolean circuit; the gate allows iff every
//! required precondition holds. If denied, the first failing precondition
//! becomes the typed refusal reason.

use crate::status::TypedStatus;
use std::collections::HashMap;
use torg_core::{evaluate, BoolOp, Graph, Node, Source};

/// Boolean state of each known precondition. Fields not enforced in this slice
/// (provenance/attestation) default to `true` so they never block the gate; the
/// Operator records evidence separately.
#[derive(Debug, Clone)]
pub struct Preconditions {
    pub weights_present: bool,
    pub weights_attested: bool,
    pub provenance_valid: bool,
    pub baseline_present: bool,
}

impl Default for Preconditions {
    fn default() -> Self {
        Self { weights_present: false, weights_attested: false, provenance_valid: true, baseline_present: false }
    }
}

impl Preconditions {
    fn value(&self, name: &str) -> Option<bool> {
        match name {
            "weights_present" => Some(self.weights_present),
            "weights_attested" => Some(self.weights_attested),
            "provenance_valid" => Some(self.provenance_valid),
            "baseline_present" => Some(self.baseline_present),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    Allow,
    Deny { reason: String },
}

impl GateDecision {
    pub fn into_status_if_denied(self) -> Option<TypedStatus> {
        match self {
            GateDecision::Allow => None,
            GateDecision::Deny { reason } => Some(TypedStatus::Refused { reason }),
        }
    }
}

/// Build a graph whose single output is the AND of `n` inputs (ids 0..n).
/// AND(a,b) = NOR(NOT a, NOT b); NOT x = NOR(x,x). Chained for n>2.
fn build_and_graph(n: usize) -> Graph {
    assert!(n >= 1, "gate requires at least one precondition");
    let inputs: Vec<u16> = (0..n as u16).collect();
    if n == 1 {
        return Graph { inputs, nodes: vec![], outputs: vec![0] };
    }
    let mut nodes = Vec::new();
    let mut next_id: u16 = n as u16;
    // running AND accumulator, starts as input 0
    let mut acc: u16 = 0;
    for i in 1..n as u16 {
        // not_acc = NOR(acc, acc)
        let not_acc = next_id; next_id += 1;
        nodes.push(Node::new(not_acc, BoolOp::Nor, Source::Id(acc), Source::Id(acc)));
        // not_i = NOR(i, i)
        let not_i = next_id; next_id += 1;
        nodes.push(Node::new(not_i, BoolOp::Nor, Source::Id(i), Source::Id(i)));
        // and = NOR(not_acc, not_i)
        let and = next_id; next_id += 1;
        nodes.push(Node::new(and, BoolOp::Nor, Source::Id(not_acc), Source::Id(not_i)));
        acc = and;
    }
    Graph { inputs, nodes, outputs: vec![acc] }
}

/// Evaluate the gate over the contract's required precondition names.
pub fn evaluate_gate(pre: &Preconditions, required: &[String]) -> GateDecision {
    if required.is_empty() {
        return GateDecision::Allow;
    }
    // Resolve each required precondition to a bool; an unknown name is a refusal.
    let mut values = Vec::with_capacity(required.len());
    for name in required {
        match pre.value(name) {
            Some(v) => values.push((name.clone(), v)),
            None => return GateDecision::Deny { reason: format!("unknown precondition: {name}") },
        }
    }
    let graph = build_and_graph(values.len());
    let mut inputs = HashMap::new();
    for (i, (_, v)) in values.iter().enumerate() {
        inputs.insert(i as u16, *v);
    }
    let out_id = *graph.outputs.first().expect("one output");
    match evaluate(&graph, &inputs) {
        Ok(result) if result.get(&out_id).copied().unwrap_or(false) => GateDecision::Allow,
        Ok(_) => {
            // Denied — name the first failing precondition.
            let failed = values.iter().find(|(_, v)| !v).map(|(n, _)| n.clone()).unwrap_or_default();
            GateDecision::Deny { reason: format!("precondition not met: {failed}") }
        }
        Err(e) => GateDecision::Deny { reason: format!("policy circuit error: {e}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> Vec<String> {
        vec!["weights_present".into(), "weights_attested".into(), "baseline_present".into()]
    }

    #[test]
    fn allows_when_all_required_true() {
        let pre = Preconditions { weights_present: true, weights_attested: true, provenance_valid: true, baseline_present: true };
        assert_eq!(evaluate_gate(&pre, &req()), GateDecision::Allow);
    }

    #[test]
    fn denies_and_names_failed_precondition() {
        let pre = Preconditions { weights_present: true, weights_attested: true, provenance_valid: true, baseline_present: false };
        match evaluate_gate(&pre, &req()) {
            GateDecision::Deny { reason } => assert!(reason.contains("baseline_present")),
            other => panic!("expected deny, got {other:?}"),
        }
    }

    #[test]
    fn unknown_precondition_is_denied() {
        let pre = Preconditions::default();
        match evaluate_gate(&pre, &["nonsense".to_string()]) {
            GateDecision::Deny { reason } => assert!(reason.contains("unknown")),
            other => panic!("expected deny, got {other:?}"),
        }
    }

    #[test]
    fn empty_required_allows() {
        assert_eq!(evaluate_gate(&Preconditions::default(), &[]), GateDecision::Allow);
    }
}
```

- [ ] **Step 2: Run tests, verify pass**

Run: `cargo nextest run -p arkavo-eval gate`
Expected: PASS (4 tests). If `torg_core`'s `Node::new` / `Source::Id` / `BoolOp`/`evaluate` names differ from these, fix imports against `crates/arkavo-torg-circuits/tests/circuit_integration.rs` (lines 54–85, 276–306) which use them verbatim.

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/gate.rs
git commit -m "arkavo-eval: TØR-G pre-flight gate over contract preconditions"
```

---

## Phase 3 — Verdict (semantic similarity + tok/s)

### Task 5: `verdict.rs` — `Embedder` trait + `assess()`

**Files:**
- Create: `crates/arkavo-eval/src/verdict.rs`

- [ ] **Step 1: Write the module + tests (with a deterministic fake embedder)**

```rust
//! Critic post-flight verdict: aggregate cosine similarity vs the baseline plus
//! a tok/s ratio. Embedding is behind a trait so the real ONNX model
//! (arkavo-memory::EmbeddingService) is only required at deploy time; tests use
//! a deterministic fake.

use crate::operator::PromptOutput;
use crate::status::TypedStatus;
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerdictError {
    #[error("embedding failed: {0}")]
    Embedding(String),
    #[error("baseline missing output for prompt {0}")]
    MissingBaselineOutput(String),
    #[error("no prompts to compare")]
    NoPrompts,
}

/// Reference outputs + aggregate tok/s captured when a baseline was blessed.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Baseline {
    pub outputs: Vec<BaselineOutput>,
    pub tok_s: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BaselineOutput {
    pub id: String,
    pub text: String,
}

impl Baseline {
    fn output_for(&self, id: &str) -> Option<&str> {
        self.outputs.iter().find(|o| o.id == id).map(|o| o.text.as_str())
    }
}

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError>;
    fn cosine(a: &[f32], b: &[f32]) -> f32
    where
        Self: Sized,
    {
        let mut dot = 0.0;
        let mut na = 0.0;
        let mut nb = 0.0;
        for i in 0..a.len().min(b.len()) {
            dot += a[i] * b[i];
            na += a[i] * a[i];
            nb += b[i] * b[i];
        }
        if na == 0.0 || nb == 0.0 {
            return 0.0;
        }
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Compute the verdict. `acceptance` carries the thresholds from the contract.
pub async fn assess<E: Embedder>(
    embed: &E,
    outputs: &[PromptOutput],
    baseline: &Baseline,
    min_similarity: f64,
    min_tok_s_ratio: f64,
) -> Result<TypedStatus, VerdictError> {
    if outputs.is_empty() {
        return Err(VerdictError::NoPrompts);
    }
    let mut sim_sum = 0.0f64;
    for o in outputs {
        let base = baseline
            .output_for(&o.id)
            .ok_or_else(|| VerdictError::MissingBaselineOutput(o.id.clone()))?;
        let va = embed.embed(&o.text).await?;
        let vb = embed.embed(base).await?;
        sim_sum += E::cosine(&va, &vb) as f64;
    }
    let mean_sim = sim_sum / outputs.len() as f64;
    if mean_sim < min_similarity {
        return Ok(TypedStatus::RegressionFailed {
            metric: "similarity".into(),
            value: mean_sim,
            threshold: min_similarity,
        });
    }
    let mean_tok_s = outputs.iter().map(|o| o.tok_s).sum::<f64>() / outputs.len() as f64;
    let ratio = if baseline.tok_s > 0.0 { mean_tok_s / baseline.tok_s } else { 1.0 };
    if ratio < min_tok_s_ratio {
        return Ok(TypedStatus::RegressionFailed {
            metric: "tok_s_ratio".into(),
            value: ratio,
            threshold: min_tok_s_ratio,
        });
    }
    Ok(TypedStatus::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic embedder: a tiny char-frequency vector. Identical text →
    /// identical vector → cosine 1.0; disjoint text → cosine ~0.
    struct FakeEmbedder;

    #[async_trait]
    impl Embedder for FakeEmbedder {
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

    fn baseline() -> Baseline {
        Baseline {
            outputs: vec![BaselineOutput { id: "p1".into(), text: "paris".into() }],
            tok_s: 100.0,
        }
    }

    #[tokio::test]
    async fn identical_output_passes() {
        let outputs = vec![PromptOutput { id: "p1".into(), text: "paris".into(), tok_s: 100.0 }];
        let s = assess(&FakeEmbedder, &outputs, &baseline(), 0.87, 0.95).await.unwrap();
        assert_eq!(s, TypedStatus::Passed);
    }

    #[tokio::test]
    async fn dissimilar_output_fails_similarity() {
        let outputs = vec![PromptOutput { id: "p1".into(), text: "zzzzz".into(), tok_s: 100.0 }];
        match assess(&FakeEmbedder, &outputs, &baseline(), 0.87, 0.95).await.unwrap() {
            TypedStatus::RegressionFailed { metric, .. } => assert_eq!(metric, "similarity"),
            other => panic!("expected regression, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn slow_output_fails_tok_s() {
        let outputs = vec![PromptOutput { id: "p1".into(), text: "paris".into(), tok_s: 50.0 }];
        match assess(&FakeEmbedder, &outputs, &baseline(), 0.87, 0.95).await.unwrap() {
            TypedStatus::RegressionFailed { metric, .. } => assert_eq!(metric, "tok_s_ratio"),
            other => panic!("expected regression, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_baseline_output_errors() {
        let outputs = vec![PromptOutput { id: "other".into(), text: "x".into(), tok_s: 100.0 }];
        assert!(matches!(
            assess(&FakeEmbedder, &outputs, &baseline(), 0.87, 0.95).await,
            Err(VerdictError::MissingBaselineOutput(_))
        ));
    }
}
```

- [ ] **Step 2: Run tests, verify pass**

Run: `cargo nextest run -p arkavo-eval verdict`
Expected: PASS (4 tests). This depends on `operator::PromptOutput` (next task). If it fails to compile, do Task 6 first, then run.

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/verdict.rs
git commit -m "arkavo-eval: semantic-similarity + tok/s verdict"
```

---

## Phase 4 — Operator trait, baseline store, planner, orchestration

### Task 6: `operator.rs` — `Operator` trait + `FakeOperator`

**Files:**
- Create: `crates/arkavo-eval/src/operator.rs`

- [ ] **Step 1: Write the module + test**

```rust
//! Operator role: runs the model over the plan's prompts and captures the
//! output text and tokens/sec per prompt. The real llama.cpp implementation
//! lands in Part 2 behind the `llama-cpp` feature; this module defines the
//! trait and a fake used by tests and the one-shot CLI demo.

use crate::plan::EvalPlan;
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OperatorError {
    #[error("model load failed: {0}")]
    Load(String),
    #[error("generation failed: {0}")]
    Generate(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptOutput {
    pub id: String,
    pub text: String,
    pub tok_s: f64,
}

#[derive(Debug, Clone)]
pub struct RunOutput {
    pub outputs: Vec<PromptOutput>,
}

#[async_trait]
pub trait Operator: Send + Sync {
    async fn run(&self, plan: &EvalPlan) -> Result<RunOutput, OperatorError>;
}

/// Returns a fixed answer per prompt id. Used by tests and the CLI demo.
pub struct FakeOperator {
    pub answers: std::collections::HashMap<String, String>,
    pub tok_s: f64,
}

#[async_trait]
impl Operator for FakeOperator {
    async fn run(&self, plan: &EvalPlan) -> Result<RunOutput, OperatorError> {
        let outputs = plan
            .prompts
            .iter()
            .map(|p| PromptOutput {
                id: p.id.clone(),
                text: self.answers.get(&p.id).cloned().unwrap_or_default(),
                tok_s: self.tok_s,
            })
            .collect();
        Ok(RunOutput { outputs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{EvalPrompt, ExecutionProfile, ModelSpec, PromptMessage};
    use std::collections::HashMap;

    #[tokio::test]
    async fn fake_operator_answers_by_id() {
        let plan = EvalPlan {
            model: ModelSpec { name: "m".into(), quant: "q".into(), weight_digest: "b3:0".into() },
            prompts: vec![EvalPrompt {
                id: "p1".into(),
                messages: vec![PromptMessage { role: "user".into(), content: "hi".into() }],
                tools: None,
            }],
            exec: ExecutionProfile { seed: 0, temperature: 0.0, threads: None, ctx: None, max_tokens: 8 },
            baseline_commit: None,
        };
        let mut answers = HashMap::new();
        answers.insert("p1".to_string(), "hello".to_string());
        let op = FakeOperator { answers, tok_s: 42.0 };
        let out = op.run(&plan).await.unwrap();
        assert_eq!(out.outputs.len(), 1);
        assert_eq!(out.outputs[0].text, "hello");
        assert_eq!(out.outputs[0].tok_s, 42.0);
    }
}
```

- [ ] **Step 2: Run tests, verify pass**

Run: `cargo nextest run -p arkavo-eval operator`
Expected: PASS (1 test). Depends on `plan::EvalPlan` (next task) — if it fails to compile, do Task 7 then re-run.

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/operator.rs
git commit -m "arkavo-eval: Operator trait + FakeOperator"
```

### Task 7: `plan.rs` — Planner

**Files:**
- Create: `crates/arkavo-eval/src/plan.rs`

- [ ] **Step 1: Write the module + test**

```rust
//! Planner role: resolves a contract into a concrete execution plan.

use crate::contract::{EvalContract, EvalPrompt, ExecutionProfile, ModelSpec};

#[derive(Debug, Clone)]
pub struct EvalPlan {
    pub model: ModelSpec,
    pub prompts: Vec<EvalPrompt>,
    pub exec: ExecutionProfile,
    /// The git commit whose baseline this run compares against (if any).
    pub baseline_commit: Option<String>,
}

pub fn plan(contract: &EvalContract) -> EvalPlan {
    EvalPlan {
        model: contract.model.clone(),
        prompts: contract.prompts.clone(),
        exec: contract.execution.clone(),
        baseline_commit: contract.baseline.commit.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::*;

    #[test]
    fn plan_carries_model_prompts_exec_and_baseline() {
        let c = EvalContract {
            contract_id: "id".into(),
            task_kind: "model_eval".into(),
            model: ModelSpec { name: "gemma-4-12b".into(), quant: "Q4_K_M".into(), weight_digest: "b3:0".into() },
            baseline: BaselineRef { kind: "reference_outputs".into(), commit: Some("c1".into()), digest: None },
            prompts: vec![EvalPrompt { id: "p1".into(), messages: vec![], tools: None }],
            acceptance: Acceptance { min_similarity: 0.87, min_tok_s_ratio: 0.95 },
            execution: ExecutionProfile { seed: 0, temperature: 0.0, threads: Some(4), ctx: Some(4096), max_tokens: 64 },
            preconditions: vec![],
            policy_circuit: "torg:x".into(),
            on_precondition_unmet: "refuse".into(),
        };
        let p = plan(&c);
        assert_eq!(p.model.name, "gemma-4-12b");
        assert_eq!(p.prompts.len(), 1);
        assert_eq!(p.exec.threads, Some(4));
        assert_eq!(p.baseline_commit.as_deref(), Some("c1"));
    }
}
```

- [ ] **Step 2: Run tests, verify pass**

Run: `cargo nextest run -p arkavo-eval plan`
Expected: PASS (1 test).

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/plan.rs
git commit -m "arkavo-eval: Planner (contract -> EvalPlan)"
```

### Task 8: `baseline.rs` — `BaselineStore` trait + `MemBaselineStore`

**Files:**
- Create: `crates/arkavo-eval/src/baseline.rs`

- [ ] **Step 1: Write the module + test**

```rust
//! Historian role: stores/retrieves baselines. The trait is backend-agnostic;
//! the TDF+iroh implementation lands in Part 2. `MemBaselineStore` is used by
//! tests and the one-shot CLI demo.

use crate::verdict::Baseline;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BaselineError {
    #[error("baseline backend error: {0}")]
    Backend(String),
}

/// A shareable pointer to a published baseline.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BaselinePointer {
    pub commit: String,
    pub model: String,
    /// `b3:<hex>` content address of the (encrypted) baseline artifact.
    pub b3_digest: String,
    /// Fetch handle (iroh ticket string in the real impl; empty for in-memory).
    pub ticket: String,
}

#[async_trait]
pub trait BaselineStore: Send + Sync {
    /// Fetch the baseline blessed at `commit` for `model`, if any.
    async fn fetch(&self, commit: &str, model: &str) -> Result<Option<Baseline>, BaselineError>;
    /// Publish `baseline` as the trusted baseline for `commit`/`model`.
    async fn publish(
        &self,
        commit: &str,
        model: &str,
        baseline: &Baseline,
    ) -> Result<BaselinePointer, BaselineError>;
}

/// In-memory store keyed by `(commit, model)`.
#[derive(Default)]
pub struct MemBaselineStore {
    inner: Mutex<HashMap<(String, String), Baseline>>,
}

impl MemBaselineStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BaselineStore for MemBaselineStore {
    async fn fetch(&self, commit: &str, model: &str) -> Result<Option<Baseline>, BaselineError> {
        Ok(self.inner.lock().unwrap().get(&(commit.to_string(), model.to_string())).cloned())
    }

    async fn publish(
        &self,
        commit: &str,
        model: &str,
        baseline: &Baseline,
    ) -> Result<BaselinePointer, BaselineError> {
        let bytes = serde_json::to_vec(baseline).map_err(|e| BaselineError::Backend(e.to_string()))?;
        let digest = crate::digest::b3_hex(&bytes);
        self.inner
            .lock()
            .unwrap()
            .insert((commit.to_string(), model.to_string()), baseline.clone());
        Ok(BaselinePointer { commit: commit.into(), model: model.into(), b3_digest: digest, ticket: String::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verdict::BaselineOutput;

    #[tokio::test]
    async fn publish_then_fetch_round_trips() {
        let store = MemBaselineStore::new();
        assert!(store.fetch("c1", "m").await.unwrap().is_none());
        let b = Baseline { outputs: vec![BaselineOutput { id: "p1".into(), text: "paris".into() }], tok_s: 100.0 };
        let ptr = store.publish("c1", "m", &b).await.unwrap();
        assert!(ptr.b3_digest.starts_with("b3:"));
        assert_eq!(store.fetch("c1", "m").await.unwrap().unwrap(), b);
    }
}
```

- [ ] **Step 2: Run tests, verify pass**

Run: `cargo nextest run -p arkavo-eval baseline`
Expected: PASS (1 test).

- [ ] **Step 3: Commit**

```bash
git add crates/arkavo-eval/src/baseline.rs
git commit -m "arkavo-eval: BaselineStore trait + in-memory store"
```

### Task 9: `lib.rs` — `run_eval()` orchestration

**Files:**
- Modify: `crates/arkavo-eval/src/lib.rs`

- [ ] **Step 1: Replace `lib.rs` with the orchestration + integration test**

```rust
//! Local-model evaluation pipeline: resolves an eval contract, gates on
//! preconditions, runs the model, and produces a typed regression verdict.

pub mod baseline;
pub mod contract;
pub mod digest;
pub mod gate;
pub mod operator;
pub mod plan;
pub mod status;
pub mod verdict;

use baseline::{BaselinePointer, BaselineStore};
use contract::EvalContract;
use gate::{evaluate_gate, Preconditions};
use operator::Operator;
use status::TypedStatus;
use verdict::{assess, Baseline, BaselineOutput, Embedder};

/// Outcome of an eval run: the terminal status plus any baseline published
/// (only on `main` runs).
#[derive(Debug)]
pub struct RunOutcome {
    pub status: TypedStatus,
    pub published: Option<BaselinePointer>,
}

/// Run the full pipeline. `is_main` is true when this run is on the default
/// branch after merge, in which case a passing/bootstrap run records the new
/// baseline.
pub async fn run_eval<O, B, E>(
    contract: &EvalContract,
    pre: &Preconditions,
    operator: &O,
    baselines: &B,
    embed: &E,
    is_main: bool,
) -> RunOutcome
where
    O: Operator,
    B: BaselineStore,
    E: Embedder,
{
    // 1. Pre-flight gate.
    if let Some(refused) = evaluate_gate(pre, &contract.preconditions).into_status_if_denied() {
        return RunOutcome { status: refused, published: None };
    }

    // 2. Plan + run the model.
    let evplan = plan::plan(contract);
    let run = match operator.run(&evplan).await {
        Ok(r) => r,
        Err(e) => {
            return RunOutcome {
                status: TypedStatus::InfraError { stage: format!("operator: {e}") },
                published: None,
            };
        }
    };

    let commit = evplan.baseline_commit.clone().unwrap_or_default();
    let model = contract.model.name.clone();

    // 3. Fetch the baseline.
    let baseline = match baselines.fetch(&commit, &model).await {
        Ok(b) => b,
        Err(e) => {
            return RunOutcome {
                status: TypedStatus::InfraError { stage: format!("historian: {e}") },
                published: None,
            };
        }
    };

    match baseline {
        // 4a. No baseline yet → bootstrap. On main, publish; on PR, neutral.
        None => {
            let new_baseline = Baseline {
                outputs: run
                    .outputs
                    .iter()
                    .map(|o| BaselineOutput { id: o.id.clone(), text: o.text.clone() })
                    .collect(),
                tok_s: mean_tok_s(&run.outputs),
            };
            let published = if is_main {
                baselines.publish(&commit, &model, &new_baseline).await.ok()
            } else {
                None
            };
            RunOutcome { status: TypedStatus::BaselineBootstrapped, published }
        }
        // 4b. Baseline exists → assess.
        Some(base) => {
            let status = match assess(
                embed,
                &run.outputs,
                &base,
                contract.acceptance.min_similarity,
                contract.acceptance.min_tok_s_ratio,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => TypedStatus::InfraError { stage: format!("verdict: {e}") },
            };
            // On a passing main run, record the new baseline (promotion on merge).
            let published = if is_main && status == TypedStatus::Passed {
                let new_baseline = Baseline {
                    outputs: run
                        .outputs
                        .iter()
                        .map(|o| BaselineOutput { id: o.id.clone(), text: o.text.clone() })
                        .collect(),
                    tok_s: mean_tok_s(&run.outputs),
                };
                baselines.publish(&commit, &model, &new_baseline).await.ok()
            } else {
                None
            };
            RunOutcome { status, published }
        }
    }
}

fn mean_tok_s(outputs: &[operator::PromptOutput]) -> f64 {
    if outputs.is_empty() {
        return 0.0;
    }
    outputs.iter().map(|o| o.tok_s).sum::<f64>() / outputs.len() as f64
}
```

- [ ] **Step 2: Add an integration test**

Create `crates/arkavo-eval/tests/pipeline.rs`:

```rust
use arkavo_eval::baseline::MemBaselineStore;
use arkavo_eval::contract::*;
use arkavo_eval::gate::Preconditions;
use arkavo_eval::operator::FakeOperator;
use arkavo_eval::status::TypedStatus;
use arkavo_eval::verdict::{Embedder, VerdictError};
use arkavo_eval::run_eval;
use async_trait::async_trait;
use std::collections::HashMap;

struct FakeEmbedder;
#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        let mut v = vec![0.0f32; 27];
        for c in text.to_lowercase().chars() {
            if c.is_ascii_lowercase() { v[(c as u8 - b'a') as usize] += 1.0; } else { v[26] += 1.0; }
        }
        Ok(v)
    }
}

fn contract(commit: &str) -> EvalContract {
    EvalContract {
        contract_id: "id".into(),
        task_kind: "model_eval".into(),
        model: ModelSpec { name: "gemma-4-12b".into(), quant: "Q4_K_M".into(), weight_digest: "b3:0".into() },
        baseline: BaselineRef { kind: "reference_outputs".into(), commit: Some(commit.into()), digest: None },
        prompts: vec![EvalPrompt {
            id: "capital".into(),
            messages: vec![PromptMessage { role: "user".into(), content: "Capital of France?".into() }],
            tools: None,
        }],
        acceptance: Acceptance { min_similarity: 0.87, min_tok_s_ratio: 0.95 },
        execution: ExecutionProfile { seed: 0, temperature: 0.0, threads: None, ctx: None, max_tokens: 32 },
        preconditions: vec!["weights_present".into(), "baseline_present".into()],
        policy_circuit: "torg:eval-preflight-v1".into(),
        on_precondition_unmet: "refuse".into(),
    }
}

fn op(answer: &str) -> FakeOperator {
    let mut a = HashMap::new();
    a.insert("capital".to_string(), answer.to_string());
    FakeOperator { answers: a, tok_s: 100.0 }
}

fn pre(baseline_present: bool) -> Preconditions {
    Preconditions { weights_present: true, weights_attested: true, provenance_valid: true, baseline_present }
}

#[tokio::test]
async fn refused_when_precondition_unmet() {
    let store = MemBaselineStore::new();
    let outcome = run_eval(&contract("c1"), &pre(false), &op("Paris"), &store, &FakeEmbedder, false).await;
    assert!(matches!(outcome.status, TypedStatus::Refused { .. }));
}

#[tokio::test]
async fn bootstraps_on_main_then_passes_on_pr() {
    let store = MemBaselineStore::new();
    // First, on main with no baseline → bootstrap + publish.
    let boot = run_eval(&contract("c1"), &pre(true), &op("Paris"), &store, &FakeEmbedder, true).await;
    assert_eq!(boot.status, TypedStatus::BaselineBootstrapped);
    assert!(boot.published.is_some());
    // Then a PR run with the same answer → passes.
    let pass = run_eval(&contract("c1"), &pre(true), &op("Paris"), &store, &FakeEmbedder, false).await;
    assert_eq!(pass.status, TypedStatus::Passed);
    assert!(pass.published.is_none());
}

#[tokio::test]
async fn regression_when_output_diverges() {
    let store = MemBaselineStore::new();
    run_eval(&contract("c1"), &pre(true), &op("Paris"), &store, &FakeEmbedder, true).await;
    let reg = run_eval(&contract("c1"), &pre(true), &op("zzzzzz"), &store, &FakeEmbedder, false).await;
    assert!(matches!(reg.status, TypedStatus::RegressionFailed { .. }));
}
```

- [ ] **Step 3: Run all crate tests, verify pass**

Run: `cargo nextest run -p arkavo-eval`
Expected: PASS (all unit tests + 3 integration tests).

- [ ] **Step 4: Lint + format**

Run: `cargo fmt -p arkavo-eval && cargo clippy -p arkavo-eval -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/arkavo-eval/src/lib.rs crates/arkavo-eval/tests/pipeline.rs
git commit -m "arkavo-eval: run_eval orchestration + pipeline integration tests"
```

---

## Phase 5 — GitHub Check Runs + one-shot CLI

### Task 10: Add Check Run methods to `arkavo-github`

**Files:**
- Modify: `crates/arkavo-github/src/operations.rs`

- [ ] **Step 1: Add the two methods inside `impl GitHubOperations`**

Locate the `add_comment` method (around `operations.rs:315`) and add these methods next to it. They mirror its exact request/auth/error pattern; `create_check_run` parses and returns the new check run `id`.

```rust
    /// Create a Check Run on `head_sha`. Returns the new check run id.
    pub async fn create_check_run(
        &self,
        owner: &str,
        repo: &str,
        name: &str,
        head_sha: &str,
        status: &str,
        conclusion: Option<&str>,
        output_title: Option<&str>,
        output_summary: Option<&str>,
    ) -> Result<u64> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/check-runs");
        let mut body = serde_json::json!({
            "name": name,
            "head_sha": head_sha,
            "status": status,
        });
        if let Some(c) = conclusion {
            body["conclusion"] = serde_json::json!(c);
        }
        if let (Some(t), Some(s)) = (output_title, output_summary) {
            body["output"] = serde_json::json!({ "title": t, "summary": s });
        }
        let resp = self
            .auth_headers(self.client.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;
        let status_code = resp.status();
        if !status_code.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Create check run failed ({status_code}): {text}"
            )));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Bad check-run response: {e}")))?;
        json["id"]
            .as_u64()
            .ok_or_else(|| GitHubError::GitHubApi("check-run response missing id".into()))
    }

    /// Update an existing Check Run (set status/conclusion/output).
    pub async fn update_check_run(
        &self,
        owner: &str,
        repo: &str,
        check_run_id: u64,
        status: &str,
        conclusion: Option<&str>,
        output_title: Option<&str>,
        output_summary: Option<&str>,
    ) -> Result<()> {
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/check-runs/{check_run_id}");
        let mut body = serde_json::json!({ "status": status });
        if let Some(c) = conclusion {
            body["conclusion"] = serde_json::json!(c);
        }
        if let (Some(t), Some(s)) = (output_title, output_summary) {
            body["output"] = serde_json::json!({ "title": t, "summary": s });
        }
        let resp = self
            .auth_headers(self.client.patch(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;
        let status_code = resp.status();
        if !status_code.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Update check run failed ({status_code}): {text}"
            )));
        }
        Ok(())
    }
```

- [ ] **Step 2: Add a test against a mock HTTP server**

Check whether `arkavo-github` already has an http-mock dev-dependency:

Run: `grep -nE 'wiremock|mockito|httpmock' crates/arkavo-github/Cargo.toml`
Expected: shows a mock lib, or nothing.

If nothing is present, prefer reusing whatever the rest of the repo uses (search `grep -rlE 'wiremock|mockito|httpmock' crates/*/Cargo.toml | head`). If a mock lib is available, add this test to a new `crates/arkavo-github/tests/check_runs.rs`. The pattern below uses `wiremock`; adapt to the repo's chosen mock lib if different:

```rust
use arkavo_github::GitHubOperations;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn create_check_run_returns_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/o/r/check-runs"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": 999 })))
        .mount(&server)
        .await;

    // GITHUB_API_BASE is compiled in; this test requires a way to point the
    // client at the mock. If GitHubOperations has no base-url override, gate
    // this test behind that refactor (see Step 3).
    let _ = server; // see note
}
```

> **Note:** `GITHUB_API_BASE` is a hardcoded const in `operations.rs`. To make these methods testable against a mock, add an optional base-url override. Do Step 3, then finish this test.

- [ ] **Step 3: Make the base URL overridable (minimal, backward-compatible)**

In `operations.rs`, find the `GitHubOperations` struct and its `new`. Add a `base_url` field defaulting to `GITHUB_API_BASE`, and a constructor for tests:

```rust
pub struct GitHubOperations {
    client: Client,
    token: String,
    base_url: String,
}

impl GitHubOperations {
    pub fn new(token: &str) -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to create HTTP client: {e}")))?;
        Ok(Self { client, token: token.to_string(), base_url: GITHUB_API_BASE.to_string() })
    }

    /// Construct with a custom API base (used by tests against a mock server).
    pub fn with_base_url(token: &str, base_url: &str) -> Result<Self> {
        let mut ops = Self::new(token)?;
        ops.base_url = base_url.to_string();
        Ok(ops)
    }
}
```

Then replace `{GITHUB_API_BASE}` with `{base}` in the two new methods, binding `let base = &self.base_url;` at the top of each. (Leave the other existing methods unchanged to keep this change minimal.)

Now finish the test in `check_runs.rs`:

```rust
    let ops = GitHubOperations::with_base_url("t", &server.uri()).unwrap();
    let id = ops.create_check_run("o", "r", "arkavo-eval/gemma-4-12b", "deadbeef", "completed", Some("success"), Some("Eval passed"), Some("ok")).await.unwrap();
    assert_eq!(id, 999);
```

And add a PATCH update test:

```rust
#[tokio::test]
async fn update_check_run_ok() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/repos/o/r/check-runs/999"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 999 })))
        .mount(&server)
        .await;
    let ops = GitHubOperations::with_base_url("t", &server.uri()).unwrap();
    ops.update_check_run("o", "r", 999, "completed", Some("failure"), Some("Regression"), Some("similarity 0.5 < 0.87")).await.unwrap();
}
```

If `wiremock` is not an acceptable dependency, instead write a unit test that asserts the JSON body construction by extracting body-building into a small `fn check_run_body(...) -> serde_json::Value` and testing that pure function — and skip the live HTTP test.

- [ ] **Step 4: Run tests + clippy**

Run: `cargo nextest run -p arkavo-github check_run && cargo clippy -p arkavo-github -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/arkavo-github/src/operations.rs crates/arkavo-github/Cargo.toml crates/arkavo-github/tests/check_runs.rs
git commit -m "arkavo-github: create/update Check Run methods"
```

### Task 11: One-shot `arkavo eval run` CLI (demo path)

**Files:**
- Create: `crates/arkavo-cli/src/commands/eval.rs`
- Modify: `crates/arkavo-cli/src/commands/mod.rs`
- Modify: `crates/arkavo-cli/src/lib.rs`
- Modify: `crates/arkavo-cli/Cargo.toml` (add `arkavo-eval` dep)

This command loads a contract JSON, runs the pipeline with a `FakeOperator` (real model arrives in Part 2) and an in-memory baseline, prints the `TypedStatus` as JSON, and exits non-zero on a failing conclusion. It proves the loop end-to-end without a model.

- [ ] **Step 1: Add the dependency**

In `crates/arkavo-cli/Cargo.toml` `[dependencies]`:

```toml
arkavo-eval = { path = "../arkavo-eval" }
```

- [ ] **Step 2: Declare the module**

In `crates/arkavo-cli/src/commands/mod.rs`, add:

```rust
pub mod eval;
```

- [ ] **Step 3: Write `crates/arkavo-cli/src/commands/eval.rs`**

```rust
//! `arkavo eval run --contract <path> [--answer id=text ...] [--main]`
//!
//! One-shot eval runner used for local verification and as the Part-2 daemon's
//! core. Uses a FakeOperator until the llama.cpp Operator lands (Part 2).

use arkavo_eval::baseline::MemBaselineStore;
use arkavo_eval::contract::EvalContract;
use arkavo_eval::gate::Preconditions;
use arkavo_eval::operator::FakeOperator;
use arkavo_eval::verdict::{Embedder, VerdictError};
use arkavo_eval::{run_eval, RunOutcome};
use async_trait::async_trait;
use std::collections::HashMap;

struct CharEmbedder;
#[async_trait]
impl Embedder for CharEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        let mut v = vec![0.0f32; 27];
        for c in text.to_lowercase().chars() {
            if c.is_ascii_lowercase() { v[(c as u8 - b'a') as usize] += 1.0; } else { v[26] += 1.0; }
        }
        Ok(v)
    }
}

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Manual arg parse (matches the repo's CLI style).
    let mut contract_path: Option<String> = None;
    let mut answers: HashMap<String, String> = HashMap::new();
    let mut is_main = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--contract" => { i += 1; contract_path = args.get(i).cloned(); }
            "--answer" => {
                i += 1;
                if let Some(kv) = args.get(i) {
                    if let Some((k, v)) = kv.split_once('=') {
                        answers.insert(k.to_string(), v.to_string());
                    }
                }
            }
            "--main" => is_main = true,
            other => return Err(format!("unknown eval arg: {other}").into()),
        }
        i += 1;
    }
    let path = contract_path.ok_or("missing --contract <path>")?;
    let contract: EvalContract = serde_json::from_str(&std::fs::read_to_string(&path)?)?;

    let rt = tokio::runtime::Runtime::new()?;
    let outcome: RunOutcome = rt.block_on(async {
        let store = MemBaselineStore::new();
        let pre = Preconditions { weights_present: true, weights_attested: true, provenance_valid: true, baseline_present: !is_main };
        let op = FakeOperator { answers, tok_s: 100.0 };
        run_eval(&contract, &pre, &op, &store, &CharEmbedder, is_main).await
    });

    println!("{}", serde_json::to_string_pretty(&outcome.status)?);
    match outcome.status.check_conclusion() {
        Some("failure") | Some("action_required") => std::process::exit(1),
        _ => Ok(()),
    }
}
```

- [ ] **Step 4: Dispatch the subcommand**

In `crates/arkavo-cli/src/lib.rs`, in the top-level `match args[0].as_str()` block (around line 64), add an arm:

```rust
        "eval" => {
            // Subcommand: only "run" is supported in Part 1.
            match args.get(1).map(|s| s.as_str()) {
                Some("run") => commands::eval::run(&args[2..]),
                _ => Err("usage: arkavo eval run --contract <path> [--answer id=text] [--main]".into()),
            }
        }
```

> Match the exact return-type/error-handling shape of the neighboring arms (some return `Result<(), Box<dyn Error>>`; if the surrounding arms wrap differently, mirror that). If `args[0]` dispatch uses a helper, follow it.

- [ ] **Step 5: Build + manual smoke test**

Create a sample contract `crates/arkavo-eval/tests/fixtures/sample_contract.json` (also reused by Part 2):

```json
{
  "contract_id": "eval/gemma-4-12b/local",
  "task_kind": "model_eval",
  "model": { "name": "gemma-4-12b", "quant": "Q4_K_M", "weight_digest": "b3:0000000000000000000000000000000000000000000000000000000000000000" },
  "baseline": { "kind": "reference_outputs", "commit": "local", "digest": null },
  "prompts": [
    { "id": "capital", "messages": [ { "role": "user", "content": "Capital of France? One word." } ] }
  ],
  "acceptance": { "min_similarity": 0.87, "min_tok_s_ratio": 0.95 },
  "execution": { "seed": 0, "temperature": 0.0, "max_tokens": 16 },
  "preconditions": ["weights_present", "baseline_present"],
  "policy_circuit": "torg:eval-preflight-v1",
  "on_precondition_unmet": "refuse"
}
```

Run (bootstrap on main): `cargo run -p arkavo -- eval run --contract crates/arkavo-eval/tests/fixtures/sample_contract.json --answer capital=Paris --main`
Expected: prints `{ "kind": "baseline_bootstrapped" }`, exits 0.

Run (PR, no baseline in this fresh in-memory store → bootstrap neutral): the in-memory store doesn't persist between processes, so a PR-mode run alone bootstraps too. This is expected for the demo; real persistence arrives in Part 2.

- [ ] **Step 6: clippy + fmt**

Run: `cargo fmt && cargo clippy -p arkavo-cli -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/arkavo-cli/Cargo.toml crates/arkavo-cli/src/commands/mod.rs crates/arkavo-cli/src/commands/eval.rs crates/arkavo-cli/src/lib.rs crates/arkavo-eval/tests/fixtures/sample_contract.json
git commit -m "arkavo-cli: one-shot 'eval run' subcommand"
```

---

## Phase 6 — Final verification (Part 1)

### Task 12: Workspace build, lint, pre-push checks

- [ ] **Step 1: Full build**

Run: `cargo build -q`
Expected: success.

- [ ] **Step 2: Format + clippy across touched crates**

Run: `cargo fmt -- --check && cargo clippy -p arkavo-eval -p arkavo-github -p arkavo-cli -- -D warnings`
Expected: clean.

- [ ] **Step 3: Run the eval + github test suites**

Run: `cargo nextest run -p arkavo-eval -p arkavo-github`
Expected: all PASS.

- [ ] **Step 4: Confirm no security regressions**

Run: `cargo test -p arkavo-cli mock_provider`
Expected: PASS (unchanged by this work; confirms the CLI crate still builds/tests).

- [ ] **Step 5: Commit any fmt fixes; push the branch when ready**

```bash
git add -A && git commit -m "arkavo-eval: fmt/clippy cleanup" || true
```

---

## Self-review notes (author)

- Each phase ends green and committable.
- Verdict and gate are deterministic and offline (fake embedder; pure torg circuit), so the gate is reproducible.
- `InfraError` is carried distinctly from `RegressionFailed` end-to-end.
- The real model Operator, TDF+iroh baseline distribution, and the org-wide daemon are Part 2 — Part 1 deliberately uses fakes so the GitHub-gating loop is provable without heavy deps.
- Deliberately trimmed from the design for this slice (noted, not silently dropped): the Scribe's full per-PR plaintext *record* artifact (contract + digests + per-prompt metrics) — the eval-state row persists the terminal verdict instead; and the Operator's `arkavo-attestation` evidence capture (recorded-not-gated). Both are small follow-ups once the gate loop is proven.
