# Local-Model Eval Gate — Implementation Plan (Part 2: Real Model, TDF+iroh Baselines, Org Daemon)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax. **This plan depends on Part 1 (`docs/local-model-eval-gate-plan.md`) being merged** — it reuses `arkavo_eval`'s traits and `run_eval()`.

**Goal:** Replace Part 1's fakes with the real backends: a llama.cpp `Operator` that runs the gemma4 prompt-set deterministically, a `BaselineStore` that publishes baselines as TDF-encrypted, iroh-distributed artifacts keyed by commit hash, and an org-wide `EvalDaemon` that polls PRs and posts the gate.

**Architecture:** Three feature-gated additions to `arkavo-eval` (`llama-cpp` → `LlamaOperator`; `embeddings` → `MemoryEmbedder`; `tdf-iroh` → `TdfIrohBaselineStore`) plus a new daemon in `arkavo-orchestrator` that reuses `arkavo-github`'s `OrgDiscovery` for repo enumeration, adds PR + changed-file polling, persists per-eval state in SQLite (extending `OrchestratorStateStore`), and drives `run_eval()`.

**Tech Stack:** `arkavo-llm` (llama.cpp FFI), `arkavo-tdf` (`OpenTdfService`, `PolicyBuilder`; mock for tests), `arkavo-tdf-iroh` (`IrohNode`, `IrohTransport`, `IrohTicket`), `arkavo-memory` (`EmbeddingService`, `OrchestratorStateStore` via `sqlx`), `arkavo-gossip` (pointer broadcast).

---

## File structure (Part 2)

`arkavo-eval` additions:

- `src/operator_llama.rs` — `LlamaOperator` (feature `llama-cpp`), `verify_weights_file()`.
- `src/embedder.rs` — `MemoryEmbedder` wrapping `EmbeddingService` (feature `embeddings`).
- `src/historian_tdf.rs` — `TdfIrohBaselineStore` (feature `tdf-iroh`) + a mock-backed round-trip test.
- `Cargo.toml` — add `llama-cpp`, `tdf-iroh` features + deps.

`arkavo-github` additions:

- `src/operations.rs` — `list_pr_files()` (changed-file paths for eligibility).

`arkavo-memory` additions:

- `src/orchestrator_state.rs` — `eval_state` table + `upsert_eval_state` / `get_eval_state`.

`arkavo-orchestrator` additions:

- `src/eval_daemon.rs` — `EvalDaemon`: discovery → PR poll → eligibility → run_eval → check + state.
- `src/lib.rs` — export `EvalDaemon`.

`arkavo-cli` additions:

- `src/commands/eval.rs` — extend with a `daemon` subcommand.
- `src/lib.rs` — dispatch `eval daemon`.

---

## Phase 1 — Real embedder

### Task 1: `embedder.rs` — `MemoryEmbedder`

**Files:**
- Create: `crates/arkavo-eval/src/embedder.rs`
- Modify: `crates/arkavo-eval/src/lib.rs` (add `#[cfg(feature="embeddings")] pub mod embedder;`)
- Modify: `crates/arkavo-eval/Cargo.toml`

- [ ] **Step 1: Confirm the EmbeddingService API**

Run: `sed -n '140,210p' crates/arkavo-memory/src/embeddings.rs`
Expected: shows `pub async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>>` and `pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32`.

- [ ] **Step 2: Write the wrapper**

```rust
//! Real `Embedder` backed by arkavo-memory's bundled offline ONNX model
//! (AllMiniLML6V2). Deterministic CPU inference → reproducible verdicts.

use crate::verdict::{Embedder, VerdictError};
use arkavo_memory::EmbeddingService;
use async_trait::async_trait;

pub struct MemoryEmbedder {
    inner: EmbeddingService,
}

impl MemoryEmbedder {
    pub fn new() -> Self {
        Self { inner: EmbeddingService::new() }
    }
}

impl Default for MemoryEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Embedder for MemoryEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        self.inner
            .generate_embedding(text)
            .await
            .map_err(|e| VerdictError::Embedding(e.to_string()))
    }
}
```

- [ ] **Step 3: Wire the feature + module**

In `crates/arkavo-eval/Cargo.toml`, the `embeddings` feature already exists from Part 1. Add to `src/lib.rs`:

```rust
#[cfg(feature = "embeddings")]
pub mod embedder;
```

- [ ] **Step 4: Build with the feature**

Run: `cargo build -p arkavo-eval --features embeddings 2>&1 | tail -20`
Expected: builds. (The ONNX model is loaded lazily at runtime, not at build time, so no model files are needed to compile.)

- [ ] **Step 5: Commit**

```bash
git add crates/arkavo-eval/src/embedder.rs crates/arkavo-eval/src/lib.rs crates/arkavo-eval/Cargo.toml
git commit -m "arkavo-eval: MemoryEmbedder wrapping the bundled ONNX embedder"
```

---

## Phase 2 — Real llama.cpp Operator

### Task 2: `verify_weights_file()` — BLAKE3 of the resident GGUF

**Files:**
- Modify: `crates/arkavo-eval/src/digest.rs`

- [ ] **Step 1: Add a streaming file-hash + test**

Append to `digest.rs`:

```rust
use std::io::Read;
use std::path::Path;

/// Stream a file through BLAKE3 and check it against a `b3:<hex>` digest.
/// Used by the Critic's `weights_attested` precondition.
pub fn verify_weights_file(path: &Path, expected_b3: &str) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("b3:{}", hasher.finalize().to_hex()) == expected_b3)
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn verifies_file_digest() {
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("arkavo-eval-weights-{}.bin", std::process::id()));
        std::fs::File::create(&tmp).unwrap().write_all(b"weights").unwrap();
        let expected = b3_hex(b"weights");
        assert!(verify_weights_file(&tmp, &expected).unwrap());
        assert!(!verify_weights_file(&tmp, &b3_hex(b"other")).unwrap());
        std::fs::remove_file(&tmp).ok();
    }
}
```

- [ ] **Step 2: Run + commit**

Run: `cargo nextest run -p arkavo-eval digest`
Expected: PASS.

```bash
git add crates/arkavo-eval/src/digest.rs
git commit -m "arkavo-eval: streaming BLAKE3 weight-file verification"
```

### Task 3: `operator_llama.rs` — `LlamaOperator`

**Files:**
- Create: `crates/arkavo-eval/src/operator_llama.rs`
- Modify: `crates/arkavo-eval/src/lib.rs`
- Modify: `crates/arkavo-eval/Cargo.toml`

- [ ] **Step 1: Confirm the provider API names**

Run:
```
grep -nE 'fn complete_with_tools|pub struct ProviderResponse|pub struct InferenceTiming|trait Provider' crates/arkavo-llm/src/*.rs crates/arkavo-llm/src/**/*.rs
```
Expected: confirms `complete_with_tools(&self, Vec<Message>, Option<Value>, Option<usize>) -> Result<ProviderResponse>`, `ProviderResponse { content, tool_calls, inference_timing, .. }`, `InferenceTiming { n_eval, generation_ms, .. }`, and which trait (`Provider`) carries `complete_with_tools`. Note the exact trait/module path for the `use` line below.

- [ ] **Step 2: Add deps + feature in `crates/arkavo-eval/Cargo.toml`**

```toml
[dependencies]
# ... existing ...
arkavo-llm = { path = "../arkavo-llm", optional = true }
serde_json = { workspace = true }

[features]
# ... existing default/embeddings ...
llama-cpp = ["dep:arkavo-llm", "arkavo-llm/llama-cpp"]
```

- [ ] **Step 3: Write the operator**

```rust
//! Real Operator: loads a GGUF via arkavo-llm's llama.cpp provider and runs the
//! prompt-set under the contract's deterministic execution profile, capturing
//! output text and tokens/sec.

use crate::operator::{Operator, OperatorError, PromptOutput, RunOutput};
use crate::plan::EvalPlan;
use arkavo_llm::{LlamaCppProvider, Message, SamplingConfig};
// NOTE: confirm in Step 1 whether `complete_with_tools` requires `use arkavo_llm::Provider;`
use async_trait::async_trait;

pub struct LlamaOperator {
    pub model_name: String,
    pub model_path: String,
}

impl LlamaOperator {
    pub fn new(model_name: impl Into<String>, model_path: impl Into<String>) -> Self {
        Self { model_name: model_name.into(), model_path: model_path.into() }
    }

    fn message(role: &str, content: &str) -> Message {
        match role {
            "system" => Message::system(content),
            "assistant" => Message::assistant(content),
            _ => Message::user(content),
        }
    }
}

#[async_trait]
impl Operator for LlamaOperator {
    async fn run(&self, plan: &EvalPlan) -> Result<RunOutput, OperatorError> {
        let config = SamplingConfig {
            temperature: plan.exec.temperature,
            seed: plan.exec.seed,
            max_tokens: plan.exec.max_tokens,
            ..Default::default()
        };
        let provider = LlamaCppProvider::new_with_config(
            self.model_name.clone(),
            self.model_path.clone(),
            None,
            config,
        )
        .map_err(|e| OperatorError::Load(e.to_string()))?;

        let mut outputs = Vec::with_capacity(plan.prompts.len());
        for prompt in &plan.prompts {
            let messages: Vec<Message> = prompt
                .messages
                .iter()
                .map(|m| Self::message(&m.role, &m.content))
                .collect();
            let resp = provider
                .complete_with_tools(messages, prompt.tools.clone(), Some(plan.exec.max_tokens as usize))
                .await
                .map_err(|e| OperatorError::Generate(e.to_string()))?;

            // Output text = content plus any tool calls (so tool-selection
            // regressions are visible to the similarity check).
            let mut text = resp.content.clone();
            for tc in &resp.tool_calls {
                text.push_str(&format!("\n[tool:{} {}]", tc.name, tc.arguments));
            }

            let tok_s = resp
                .inference_timing
                .as_ref()
                .filter(|t| t.generation_ms > 0.0)
                .map(|t| t.n_eval as f64 / (t.generation_ms / 1000.0))
                .unwrap_or(0.0);

            outputs.push(PromptOutput { id: prompt.id.clone(), text, tok_s });
        }
        Ok(RunOutput { outputs })
    }
}
```

- [ ] **Step 4: Wire the module**

In `src/lib.rs`:

```rust
#[cfg(feature = "llama-cpp")]
pub mod operator_llama;
```

- [ ] **Step 5: Add an `#[ignore]` live test (requires the model)**

Create `crates/arkavo-eval/tests/llama_operator.rs`:

```rust
#![cfg(feature = "llama-cpp")]

use arkavo_eval::contract::{EvalPrompt, ExecutionProfile, ModelSpec, PromptMessage};
use arkavo_eval::operator::Operator;
use arkavo_eval::operator_llama::LlamaOperator;
use arkavo_eval::plan::EvalPlan;

fn find_gemma4_12b() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let dir = format!("{home}/.cache/huggingface/hub/models--ggml-org--gemma-4-12B-it-GGUF");
    for snap in std::fs::read_dir(format!("{dir}/snapshots")).ok()?.flatten() {
        let gguf = snap.path().join("gemma-4-12B-it-Q4_K_M.gguf");
        if gguf.exists() {
            return Some(gguf.to_string_lossy().to_string());
        }
    }
    None
}

#[tokio::test]
#[ignore = "requires local Gemma-4-12B model (~7GB)"]
async fn runs_prompt_and_reports_tok_s() {
    let Some(path) = find_gemma4_12b() else {
        eprintln!("skip: model not present");
        return;
    };
    let op = LlamaOperator::new("gemma-4-12b", path);
    let plan = EvalPlan {
        model: ModelSpec { name: "gemma-4-12b".into(), quant: "Q4_K_M".into(), weight_digest: "b3:0".into() },
        prompts: vec![EvalPrompt {
            id: "capital".into(),
            messages: vec![PromptMessage { role: "user".into(), content: "Capital of France? One word.".into() }],
            tools: None,
        }],
        exec: ExecutionProfile { seed: 0, temperature: 0.0, threads: None, ctx: None, max_tokens: 16 },
        baseline_commit: None,
    };
    let out = op.run(&plan).await.unwrap();
    assert_eq!(out.outputs.len(), 1);
    assert!(out.outputs[0].text.to_lowercase().contains("paris"));
    assert!(out.outputs[0].tok_s > 0.0);
}
```

- [ ] **Step 6: Build with feature; run the live test locally if the model is present**

Run: `cargo build -p arkavo-eval --features llama-cpp 2>&1 | tail -20`
Expected: builds (fix the `use` for the `Provider` trait if `complete_with_tools` is a trait method — per Step 1).

Run (only on a machine with the model): `cargo nextest run -p arkavo-eval --features llama-cpp -- --ignored runs_prompt`
Expected: PASS, prints a "paris" answer.

- [ ] **Step 7: clippy + commit**

Run: `cargo clippy -p arkavo-eval --features llama-cpp -- -D warnings`

```bash
git add crates/arkavo-eval/src/operator_llama.rs crates/arkavo-eval/src/lib.rs crates/arkavo-eval/Cargo.toml crates/arkavo-eval/tests/llama_operator.rs
git commit -m "arkavo-eval: real llama.cpp Operator"
```

---

## Phase 3 — TDF + iroh baseline distribution (Historian)

### Task 4: `historian_tdf.rs` — `TdfIrohBaselineStore`

**Files:**
- Create: `crates/arkavo-eval/src/historian_tdf.rs`
- Modify: `crates/arkavo-eval/src/lib.rs`
- Modify: `crates/arkavo-eval/Cargo.toml`

The store is generic over a TDF service (`TdfEncryptor + TdfDecryptor`) so tests use a mock and production wires `OpenTdfService` + a KAS-backed decryptor. It keeps a local copy of each encrypted manifest (for re-staging on restart) and a JSON index `commit/model → {b3, ticket}`.

- [ ] **Step 1: Add deps + feature in `crates/arkavo-eval/Cargo.toml`**

```toml
[dependencies]
# ... existing ...
arkavo-tdf = { path = "../arkavo-tdf", optional = true }
arkavo-tdf-iroh = { path = "../arkavo-tdf-iroh", optional = true }
tokio = { workspace = true, optional = true }

[features]
tdf-iroh = ["dep:arkavo-tdf", "dep:arkavo-tdf-iroh", "dep:tokio"]
```

- [ ] **Step 2: Write the store**

```rust
//! Historian backend that publishes baselines as TDF-encrypted, iroh-distributed
//! artifacts keyed by commit hash. The encrypted manifest is content-addressed
//! (`b3:<hex>`); other agents resolve a commit → ticket via the index, fetch the
//! ciphertext over iroh, TDF-decrypt under their capability, and verify the b3
//! digest before trusting it.

use crate::baseline::{BaselineError, BaselinePointer, BaselineStore};
use crate::digest::b3_hex;
use crate::verdict::Baseline;
use arkavo_tdf::{Policy, TdfDecryptor, TdfEncryptor, TdfManifest};
use arkavo_tdf_iroh::IrohTransport;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct StoredPtr {
    b3_digest: String,
    ticket: String,
}

pub struct TdfIrohBaselineStore<S>
where
    S: TdfEncryptor + TdfDecryptor + Send + Sync,
{
    tdf: S,
    transport: IrohTransport,
    policy: Policy,
    local_dir: PathBuf,
    index: Mutex<HashMap<(String, String), StoredPtr>>,
}

impl<S> TdfIrohBaselineStore<S>
where
    S: TdfEncryptor + TdfDecryptor + Send + Sync,
{
    /// Create the store, loading any persisted index from `local_dir/index.json`.
    pub fn new(tdf: S, transport: IrohTransport, policy: Policy, local_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&local_dir).ok();
        let index = std::fs::read_to_string(local_dir.join("index.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, StoredPtr>>(&s).ok())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| {
                        let (commit, model) = k.split_once('\u{1f}')?;
                        Some(((commit.to_string(), model.to_string()), v))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { tdf, transport, policy, local_dir, index: Mutex::new(index) }
    }

    fn persist_index(&self) {
        let flat: HashMap<String, StoredPtr> = self
            .index
            .lock()
            .unwrap()
            .iter()
            .map(|((c, m), v)| (format!("{c}\u{1f}{m}"), v.clone()))
            .collect();
        if let Ok(json) = serde_json::to_string(&flat) {
            std::fs::write(self.local_dir.join("index.json"), json).ok();
        }
    }

    fn manifest_path(&self, b3: &str) -> PathBuf {
        self.local_dir.join(format!("{}.tdf.json", b3.replace(':', "_")))
    }
}

#[async_trait]
impl<S> BaselineStore for TdfIrohBaselineStore<S>
where
    S: TdfEncryptor + TdfDecryptor + Send + Sync,
{
    async fn fetch(&self, commit: &str, model: &str) -> Result<Option<Baseline>, BaselineError> {
        let ptr = self
            .index
            .lock()
            .unwrap()
            .get(&(commit.to_string(), model.to_string()))
            .cloned();
        let Some(ptr) = ptr else { return Ok(None) };

        // Prefer the local copy; fall back to iroh fetch by ticket.
        let manifest_bytes = match std::fs::read(self.manifest_path(&ptr.b3_digest)) {
            Ok(b) => b,
            Err(_) => {
                let ticket: arkavo_tdf_iroh::IrohTicket =
                    ptr.ticket.parse().map_err(|e| BaselineError::Backend(format!("ticket: {e}")))?;
                self.transport
                    .fetch_bytes(&ticket)
                    .await
                    .map_err(|e| BaselineError::Backend(format!("iroh fetch: {e}")))?
            }
        };
        // Integrity: the content address is over the encrypted manifest bytes.
        if b3_hex(&manifest_bytes) != ptr.b3_digest {
            return Err(BaselineError::Backend("baseline digest mismatch".into()));
        }
        let manifest: TdfManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|e| BaselineError::Backend(e.to_string()))?;
        let plaintext = self
            .tdf
            .decrypt(&manifest)
            .await
            .map_err(|e| BaselineError::Backend(format!("tdf decrypt: {e}")))?;
        let baseline: Baseline =
            serde_json::from_slice(&plaintext).map_err(|e| BaselineError::Backend(e.to_string()))?;
        Ok(Some(baseline))
    }

    async fn publish(
        &self,
        commit: &str,
        model: &str,
        baseline: &Baseline,
    ) -> Result<BaselinePointer, BaselineError> {
        let plaintext = serde_json::to_vec(baseline).map_err(|e| BaselineError::Backend(e.to_string()))?;
        let manifest = self
            .tdf
            .encrypt(&plaintext, &self.policy)
            .await
            .map_err(|e| BaselineError::Backend(format!("tdf encrypt: {e}")))?;
        let manifest_bytes =
            serde_json::to_vec(&manifest).map_err(|e| BaselineError::Backend(e.to_string()))?;
        let b3 = b3_hex(&manifest_bytes);

        // Local copy (re-stageable on restart) + iroh stage for distribution.
        std::fs::write(self.manifest_path(&b3), &manifest_bytes)
            .map_err(|e| BaselineError::Backend(e.to_string()))?;
        let ticket = self
            .transport
            .stage_bytes(&manifest_bytes)
            .await
            .map_err(|e| BaselineError::Backend(format!("iroh stage: {e}")))?;
        let ticket_str = ticket.to_string();

        self.index.lock().unwrap().insert(
            (commit.to_string(), model.to_string()),
            StoredPtr { b3_digest: b3.clone(), ticket: ticket_str.clone() },
        );
        self.persist_index();

        Ok(BaselinePointer { commit: commit.into(), model: model.into(), b3_digest: b3, ticket: ticket_str })
    }
}
```

- [ ] **Step 3: Wire the module**

In `src/lib.rs`:

```rust
#[cfg(feature = "tdf-iroh")]
pub mod historian_tdf;
```

- [ ] **Step 4: Round-trip test with a mock TDF service + in-memory iroh**

Create `crates/arkavo-eval/tests/historian_tdf.rs`:

```rust
#![cfg(feature = "tdf-iroh")]

use arkavo_eval::baseline::BaselineStore;
use arkavo_eval::historian_tdf::TdfIrohBaselineStore;
use arkavo_eval::verdict::{Baseline, BaselineOutput};
use arkavo_tdf::{
    EncryptionInformation, EncryptionMethod, InlinePayload, Policy, TdfDecryptor, TdfEncryptor,
    TdfError, TdfManifest,
};
use arkavo_tdf_iroh::{IrohNode, IrohTransport};
use async_trait::async_trait;
use base64::Engine;

/// Trivial XOR "cipher" purely to exercise the encrypt→stage→fetch→decrypt path
/// offline. Production uses OpenTdfService + KAS (see Task 8).
struct MockTdf(u8);

#[async_trait]
impl TdfEncryptor for MockTdf {
    async fn encrypt(&self, plaintext: &[u8], _policy: &Policy) -> Result<TdfManifest, TdfError> {
        let ct: Vec<u8> = plaintext.iter().map(|b| b ^ self.0).collect();
        let b64 = base64::engine::general_purpose::STANDARD.encode(ct);
        Ok(TdfManifest::new(
            EncryptionInformation {
                key_type: "mock".into(),
                key_access: vec![],
                method: EncryptionMethod { algorithm: "xor".into(), iv: String::new(), is_streamable: false },
                policy: String::new(),
            },
            InlinePayload::binary(&b64),
        ))
    }
    async fn encrypt_stream<R>(&self, _r: R, _p: &Policy) -> Result<TdfManifest, TdfError>
    where
        R: tokio::io::AsyncRead + Send + Unpin,
    {
        unreachable!()
    }
}

#[async_trait]
impl TdfDecryptor for MockTdf {
    async fn decrypt(&self, manifest: &TdfManifest) -> Result<Vec<u8>, TdfError> {
        let ct = base64::engine::general_purpose::STANDARD
            .decode(&manifest.payload.value)
            .map_err(|e| TdfError::Decryption(e.to_string()))?;
        Ok(ct.iter().map(|b| b ^ self.0).collect())
    }
    async fn decrypt_stream<W>(&self, _m: &TdfManifest, _w: W) -> Result<u64, TdfError>
    where
        W: tokio::io::AsyncWrite + Send + Unpin,
    {
        unreachable!()
    }
}

#[tokio::test]
async fn publish_then_fetch_round_trips_through_tdf_and_iroh() {
    let node = IrohNode::memory().await.unwrap();
    let transport = IrohTransport::new(node.clone());
    let dir = std::env::temp_dir().join(format!("arkavo-eval-baselines-{}", std::process::id()));
    let store = TdfIrohBaselineStore::new(MockTdf(0x5A), transport, Policy::default(), dir.clone());

    let baseline = Baseline {
        outputs: vec![BaselineOutput { id: "p1".into(), text: "paris".into() }],
        tok_s: 100.0,
    };
    let ptr = store.publish("c1", "gemma-4-12b", &baseline).await.unwrap();
    assert!(ptr.b3_digest.starts_with("b3:"));
    assert!(!ptr.ticket.is_empty());

    let fetched = store.fetch("c1", "gemma-4-12b").await.unwrap().unwrap();
    assert_eq!(fetched, baseline);

    node.stop().await.unwrap();
    std::fs::remove_dir_all(dir).ok();
}
```

> If `TdfError`'s variants differ (e.g. it's `TdfError::Decrypt` not `Decryption`), fix against `crates/arkavo-tdf/src/error.rs`. If `base64` isn't a dev-dep of `arkavo-eval`, add it under `[dev-dependencies]` matching the workspace version.

- [ ] **Step 5: Build + run the round-trip test**

Run: `cargo nextest run -p arkavo-eval --features tdf-iroh historian`
Expected: PASS. Confirms encrypt → b3-address → iroh stage → fetch → decrypt → equal.

- [ ] **Step 6: clippy + commit**

Run: `cargo clippy -p arkavo-eval --features tdf-iroh -- -D warnings`

```bash
git add crates/arkavo-eval/src/historian_tdf.rs crates/arkavo-eval/src/lib.rs crates/arkavo-eval/Cargo.toml crates/arkavo-eval/tests/historian_tdf.rs
git commit -m "arkavo-eval: TDF-encrypted, iroh-distributed baseline store"
```

### Task 5: Document the production TDF wiring (no code yet — config doc)

**Files:**
- Modify: `docs/local-model-eval-gate-design.md` (append an "Operational wiring" note) — or a new `docs/eval-baseline-tdf-wiring.md`.

- [ ] **Step 1: Record the exact production constructors**

Add this note so the daemon task (Task 9) wires real TDF:

```
Production baseline store:
  let tdf = arkavo_tdf::OpenTdfService::with_kas_url(kas_url);      // feature "opentdf"
  // Decrypt path requires KAS rewrap; wire arkavo_tdf::ArkavoKasClient (feature "kas")
  // as the decryptor, or a service that composes OpenTdfService + ArkavoKasClient.
  let policy = arkavo_tdf::PolicyBuilder::new()
      .id("eval-baseline")
      .attribute(arkavo_tdf::policy::arkavo_attrs::ORGANIZATION, &["arkavo"])
      .dissemination(&[ /* swarm agent DIDs authorized to decrypt */ ])
      .build()?;
  let node = arkavo_tdf_iroh::IrohNode::memory().await?;            // or a persistent node
  let transport = arkavo_tdf_iroh::IrohTransport::new(node);
  let store = TdfIrohBaselineStore::new(tdf, transport, policy, baseline_dir);

Verify before adopting: `OpenTdfService`/`ArkavoKasClient` build rustls-only (they do,
via opentdf-rs `rustls-tls`), and the no-OpenSSL CI check still passes with the
`opentdf`/`kas` features on the daemon binary.
```

- [ ] **Step 2: Commit**

```bash
git add docs/
git commit -m "Document production TDF+KAS baseline wiring"
```

---

## Phase 4 — Changed-file eligibility helper

### Task 6: `list_pr_files()` on `GitHubOperations`

**Files:**
- Modify: `crates/arkavo-github/src/operations.rs`

- [ ] **Step 1: Add the method (mirrors the add_comment pattern; paginates one page of up to 100)**

```rust
    /// List the file paths changed in a pull request (first 100 files).
    pub async fn list_pr_files(&self, owner: &str, repo: &str, number: u64) -> Result<Vec<String>> {
        let base = &self.base_url;
        let url = format!("{base}/repos/{owner}/{repo}/pulls/{number}/files?per_page=100");
        let resp = self
            .auth_headers(self.client.get(&url))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!("List PR files failed ({status}): {text}")));
        }
        let files: Vec<serde_json::Value> =
            resp.json().await.map_err(|e| GitHubError::GitHubApi(format!("Bad files response: {e}")))?;
        Ok(files.into_iter().filter_map(|f| f["filename"].as_str().map(String::from)).collect())
    }
```

> This uses `self.base_url` added in Part 1, Task 10 Step 3.

- [ ] **Step 2: Mock test**

Add to `crates/arkavo-github/tests/check_runs.rs`:

```rust
#[tokio::test]
async fn list_pr_files_returns_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/pulls/7/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "filename": "crates/arkavo-llm/src/lib.rs" },
            { "filename": "README.md" }
        ])))
        .mount(&server)
        .await;
    let ops = GitHubOperations::with_base_url("t", &server.uri()).unwrap();
    let files = ops.list_pr_files("o", "r", 7).await.unwrap();
    assert_eq!(files, vec!["crates/arkavo-llm/src/lib.rs", "README.md"]);
}
```

- [ ] **Step 3: Run + commit**

Run: `cargo nextest run -p arkavo-github`

```bash
git add crates/arkavo-github/src/operations.rs crates/arkavo-github/tests/check_runs.rs
git commit -m "arkavo-github: list_pr_files for eligibility checks"
```

---

## Phase 5 — Persistent eval state

### Task 7: `eval_state` table on `OrchestratorStateStore`

**Files:**
- Modify: `crates/arkavo-memory/src/orchestrator_state.rs`

- [ ] **Step 1: Read the existing schema-init + a query method to mirror exactly**

Run: `sed -n '87,300p' crates/arkavo-memory/src/orchestrator_state.rs`
Expected: shows `init_schema`, `sqlx::query(...).execute(&self.pool)`, and a `query_as` example.

- [ ] **Step 2: Add an `eval_state` table inside `init_schema` (after the existing `CREATE TABLE`s)**

```rust
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS eval_state (
                repo TEXT NOT NULL,
                head_sha TEXT NOT NULL,
                model TEXT NOT NULL,
                status_json TEXT NOT NULL,
                check_run_id INTEGER,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (repo, head_sha, model)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
```

- [ ] **Step 3: Add upsert + get methods on the same `impl OrchestratorStateStore`**

```rust
    /// Record (or update) the terminal status + check run id for one eval.
    pub async fn upsert_eval_state(
        &self,
        repo: &str,
        head_sha: &str,
        model: &str,
        status_json: &str,
        check_run_id: Option<i64>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO eval_state (repo, head_sha, model, status_json, check_run_id, updated_at)
            VALUES (?, ?, ?, ?, ?, datetime('now'))
            ON CONFLICT(repo, head_sha, model) DO UPDATE SET
                status_json = excluded.status_json,
                check_run_id = excluded.check_run_id,
                updated_at = datetime('now')
            "#,
        )
        .bind(repo)
        .bind(head_sha)
        .bind(model)
        .bind(status_json)
        .bind(check_run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Returns (status_json, check_run_id) if this eval has been recorded.
    pub async fn get_eval_state(
        &self,
        repo: &str,
        head_sha: &str,
        model: &str,
    ) -> Result<Option<(String, Option<i64>)>> {
        let row = sqlx::query_as::<_, (String, Option<i64>)>(
            r#"
            SELECT status_json, check_run_id FROM eval_state
            WHERE repo = ? AND head_sha = ? AND model = ?
            "#,
        )
        .bind(repo)
        .bind(head_sha)
        .bind(model)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
```

> Match `Result`'s error type to the rest of the file (it uses the crate's `Result`/`MemoryError`). If `init_schema` is named differently (e.g. `ensure_table_exists`), add the `CREATE TABLE` there.

- [ ] **Step 4: Add a test (mirror an existing async storage test in that file)**

Add under the file's `#[cfg(test)] mod tests`:

```rust
    #[tokio::test]
    async fn eval_state_round_trips() {
        let dir = std::env::temp_dir().join(format!("arkavo-eval-state-{}", std::process::id()));
        let store = OrchestratorStateStore::new(&dir.join("state.db")).await.unwrap();
        assert!(store.get_eval_state("o/r", "sha1", "gemma-4-12b").await.unwrap().is_none());
        store
            .upsert_eval_state("o/r", "sha1", "gemma-4-12b", r#"{"kind":"passed"}"#, Some(42))
            .await
            .unwrap();
        let got = store.get_eval_state("o/r", "sha1", "gemma-4-12b").await.unwrap().unwrap();
        assert_eq!(got.0, r#"{"kind":"passed"}"#);
        assert_eq!(got.1, Some(42));
        std::fs::remove_dir_all(dir).ok();
    }
```

> Confirm the `OrchestratorStateStore::new` signature from the file (Part 1 exploration shows `new(db_path: &Path)`); adjust the test if it differs.

- [ ] **Step 5: Run + commit**

Run: `cargo nextest run -p arkavo-memory eval_state`

```bash
git add crates/arkavo-memory/src/orchestrator_state.rs
git commit -m "arkavo-memory: eval_state table for eval idempotency"
```

---

## Phase 6 — The org-wide EvalDaemon

### Task 8: Eligibility logic (pure, unit-tested)

**Files:**
- Create: `crates/arkavo-orchestrator/src/eval_eligibility.rs`
- Modify: `crates/arkavo-orchestrator/src/lib.rs`

- [ ] **Step 1: Write the pure eligibility function + tests**

```rust
//! Decide whether a PR's changed files (and labels) make it eligible for a
//! local-model eval. Pure logic, unit-tested without GitHub.

/// Default model-behavior path prefixes that trigger an eval.
pub const MODEL_PATHS: &[&str] = &[
    "crates/arkavo-llm/",
    "crates/arkavo-llama-cpp/",
    "crates/arkavo-llama-cpp-sys/",
    "vendor/llama.cpp/",
    "crates/arkavo-router/src/decision.rs",
    "crates/arkavo-torg/",
];

pub fn is_eligible(changed_files: &[String], labels: &[String]) -> bool {
    if labels.iter().any(|l| l == "eval:skip") {
        return false;
    }
    if labels.iter().any(|l| l == "eval:local-models") {
        return true;
    }
    changed_files
        .iter()
        .any(|f| MODEL_PATHS.iter().any(|p| f.starts_with(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_model_paths() {
        assert!(is_eligible(&["crates/arkavo-llm/src/lib.rs".into()], &[]));
        assert!(!is_eligible(&["README.md".into()], &[]));
    }

    #[test]
    fn labels_override() {
        assert!(is_eligible(&["README.md".into()], &["eval:local-models".into()]));
        assert!(!is_eligible(
            &["crates/arkavo-llm/src/lib.rs".into()],
            &["eval:skip".into()]
        ));
    }
}
```

- [ ] **Step 2: Export + test + commit**

In `crates/arkavo-orchestrator/src/lib.rs` add `pub mod eval_eligibility;`.

Run: `cargo nextest run -p arkavo-orchestrator eligib`

```bash
git add crates/arkavo-orchestrator/src/eval_eligibility.rs crates/arkavo-orchestrator/src/lib.rs
git commit -m "arkavo-orchestrator: PR eval eligibility logic"
```

### Task 9: `EvalDaemon` — discovery → poll → run → report

**Files:**
- Create: `crates/arkavo-orchestrator/src/eval_daemon.rs`
- Modify: `crates/arkavo-orchestrator/src/lib.rs`
- Modify: `crates/arkavo-orchestrator/Cargo.toml` (add `arkavo-eval` with `embeddings,llama-cpp,tdf-iroh`)

- [ ] **Step 1: Confirm the repo-enumeration + PR-list APIs**

Run:
```
grep -nE 'pub fn|pub async fn' crates/arkavo-github/src/org_discovery.rs 2>/dev/null | head -30
grep -nE 'pub async fn list_prs|struct GitHubPullRequest' crates/arkavo-github/src/operations.rs
```
Expected: an `OrgDiscovery` method that returns repos for an org, and `list_prs(owner, repo, ...) -> Vec<GitHubPullRequest>` with a `head` SHA + `number` + `labels`. Note the exact names/fields for the code below and adjust.

- [ ] **Step 2: Add the dependency**

In `crates/arkavo-orchestrator/Cargo.toml`:

```toml
arkavo-eval = { path = "../arkavo-eval", features = ["embeddings", "llama-cpp", "tdf-iroh"] }
```

- [ ] **Step 3: Write the daemon**

This wires the real backends and loops with bounded concurrency + per-repo error isolation. Adjust `OrgDiscovery`/`list_prs` calls to the exact names from Step 1.

```rust
//! Org-wide eval daemon: enumerates repos, polls open PRs, and runs the
//! local-model eval gate, posting a GitHub Check Run per (PR, model). Reuses
//! arkavo-github discovery/auth and arkavo-eval's pipeline.

use crate::eval_eligibility::is_eligible;
use arkavo_eval::baseline::BaselineStore;
use arkavo_eval::contract::*;
use arkavo_eval::digest::verify_weights_file;
use arkavo_eval::embedder::MemoryEmbedder;
use arkavo_eval::gate::Preconditions;
use arkavo_eval::operator_llama::LlamaOperator;
use arkavo_eval::run_eval;
use arkavo_github::{CheckRunDetails, GitHubApp, GitHubOperations};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Models run on every eligible PR (default + smallest). 26B/31B are a nightly
/// concern handled separately.
const PR_MODELS: &[&str] = &["gemma-4-12b", "gemma-4-E2B"];

pub struct EvalDaemon<B: BaselineStore + 'static> {
    org: String,
    app: Arc<GitHubApp>,
    baselines: Arc<B>,
    embedder: Arc<MemoryEmbedder>,
    state: Arc<arkavo_memory::OrchestratorStateStore>,
    poll_interval: Duration,
    max_concurrent: usize,
    /// Resolves a model name → on-disk GGUF path + expected b3 weight digest.
    resolve_model: Arc<dyn Fn(&str) -> Option<(String, String)> + Send + Sync>,
}

impl<B: BaselineStore + 'static> EvalDaemon<B> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        org: String,
        app: Arc<GitHubApp>,
        baselines: Arc<B>,
        embedder: Arc<MemoryEmbedder>,
        state: Arc<arkavo_memory::OrchestratorStateStore>,
        poll_interval: Duration,
        max_concurrent: usize,
        resolve_model: Arc<dyn Fn(&str) -> Option<(String, String)> + Send + Sync>,
    ) -> Self {
        Self { org, app, baselines, embedder, state, poll_interval, max_concurrent, resolve_model }
    }

    /// Run forever (until the process is stopped).
    pub async fn run(&self) {
        loop {
            if let Err(e) = self.poll_once().await {
                tracing::error!("eval daemon poll error: {e}");
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    async fn poll_once(&self) -> Result<(), String> {
        // 1. Enumerate repos (adjust to the real OrgDiscovery method from Step 1).
        let repos = self.discover_repos().await?;
        let sem = Arc::new(Semaphore::new(self.max_concurrent));
        let mut handles = Vec::new();
        for repo in repos {
            let permit = sem.clone().acquire_owned().await.unwrap();
            let this = self.clone_refs();
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                if let Err(e) = this.poll_repo(&repo).await {
                    // Per-repo error isolation: log and continue.
                    tracing::warn!("repo {repo} eval poll failed: {e}");
                }
            }));
        }
        for h in handles {
            let _ = h.await;
        }
        Ok(())
    }

    async fn poll_repo(&self, repo: &str) -> Result<(), String> {
        let (owner, name) = repo.split_once('/').ok_or("repo must be owner/name")?;
        let token = self.app.get_token(owner, name).await.map_err(|e| e.to_string())?;
        let ops = GitHubOperations::new(&token).map_err(|e| e.to_string())?;

        // 2. List open PRs (adjust to the real list_prs signature from Step 1).
        let prs = ops.list_prs(owner, name).await.map_err(|e| e.to_string())?;
        for pr in prs {
            let files = ops
                .list_pr_files(owner, name, pr.number)
                .await
                .map_err(|e| e.to_string())?;
            let labels: Vec<String> = pr.labels.clone();
            if !is_eligible(&files, &labels) {
                continue;
            }
            for model in PR_MODELS {
                self.eval_pr_model(&ops, owner, name, &pr.head_sha, model).await?;
            }
        }
        Ok(())
    }

    async fn eval_pr_model(
        &self,
        ops: &GitHubOperations,
        owner: &str,
        name: &str,
        head_sha: &str,
        model: &str,
    ) -> Result<(), String> {
        let repo = format!("{owner}/{name}");
        // 3. Idempotency: skip if already recorded for this head SHA + model.
        if self
            .state
            .get_eval_state(&repo, head_sha, model)
            .await
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Ok(());
        }

        let check_name = format!("arkavo-eval/{model}");
        // 4. Post a queued check.
        let check_id = ops
            .create_check_run(owner, name, &check_name, head_sha, "in_progress", CheckRunDetails::default())
            .await
            .map_err(|e| e.to_string())?;

        // 5. Resolve weights + build the contract.
        let Some((gguf_path, weight_digest)) = (self.resolve_model)(model) else {
            let summary = format!("model {model} not resident on this swarm member");
            ops.update_check_run(owner, name, check_id, "completed", CheckRunDetails { conclusion: Some("neutral"), output_title: Some("Skipped"), output_summary: Some(&summary) })
                .await
                .map_err(|e| e.to_string())?;
            return Ok(());
        };
        let contract = build_contract(model, &weight_digest, head_sha);

        // 6. Preconditions: real weight verify; provenance not-enforced (true).
        let weights_present = std::path::Path::new(&gguf_path).exists();
        let weights_attested = weights_present
            && verify_weights_file(std::path::Path::new(&gguf_path), &weight_digest).unwrap_or(false);
        let baseline_present = self
            .baselines
            .fetch(head_sha_baseline_commit(&contract), model)
            .await
            .map_err(|e| e.to_string())?
            .is_some();
        let pre = Preconditions { weights_present, weights_attested, provenance_valid: true, baseline_present };

        // 7. Run the pipeline. PR runs are never `is_main`.
        let operator = LlamaOperator::new(model.to_string(), gguf_path);
        let outcome = run_eval(&contract, &pre, &operator, self.baselines.as_ref(), self.embedder.as_ref(), false).await;

        // 8. Update the check + persist state.
        let conclusion = outcome.status.check_conclusion().unwrap_or("neutral");
        let title = outcome.status.summary();
        ops.update_check_run(owner, name, check_id, "completed", CheckRunDetails { conclusion: Some(conclusion), output_title: Some(&title), output_summary: Some(&title) })
            .await
            .map_err(|e| e.to_string())?;
        let status_json = serde_json::to_string(&outcome.status).unwrap_or_default();
        self.state
            .upsert_eval_state(&repo, head_sha, model, &status_json, Some(check_id as i64))
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// The baseline commit a PR compares against. For the working slice we anchor to
/// a fixed ref name; refine to the merge-base in a follow-up.
fn head_sha_baseline_commit(contract: &EvalContract) -> &str {
    contract.baseline.commit.as_deref().unwrap_or("main")
}

/// Build a per-model eval contract for a PR head SHA. The prompt-set is the
/// gemma4 capability tasks; thresholds inherit the design defaults.
fn build_contract(model: &str, weight_digest: &str, head_sha: &str) -> EvalContract {
    EvalContract {
        contract_id: format!("eval/{model}/{head_sha}"),
        task_kind: "model_eval".into(),
        model: ModelSpec { name: model.into(), quant: "Q4_K_M".into(), weight_digest: weight_digest.into() },
        baseline: BaselineRef { kind: "reference_outputs".into(), commit: Some("main".into()), digest: None },
        prompts: capability_prompts(),
        acceptance: Acceptance { min_similarity: 0.87, min_tok_s_ratio: 0.95 },
        execution: ExecutionProfile { seed: 0, temperature: 0.0, threads: None, ctx: None, max_tokens: 128 },
        preconditions: vec!["weights_present".into(), "weights_attested".into(), "baseline_present".into()],
        policy_circuit: "torg:eval-preflight-v1".into(),
        on_precondition_unmet: "refuse".into(),
    }
}

/// The capability prompt-set, mirrored from gemma4_compare_test.rs.
fn capability_prompts() -> Vec<EvalPrompt> {
    let weather_tools = serde_json::json!([
        { "name": "get_weather", "description": "Weather for a location",
          "input_schema": { "type": "object", "properties": { "location": { "type": "string" } }, "required": ["location"] } }
    ]);
    vec![
        EvalPrompt { id: "restraint".into(),
            messages: vec![PromptMessage { role: "user".into(), content: "What is the capital of France? Answer in one word.".into() }],
            tools: None },
        EvalPrompt { id: "tool_call".into(),
            messages: vec![PromptMessage { role: "user".into(), content: "What's the weather in New York?".into() }],
            tools: Some(weather_tools) },
        EvalPrompt { id: "reasoning".into(),
            messages: vec![PromptMessage { role: "user".into(), content: "A car travels 140 km in 2 hours. Average speed in km/h?".into() }],
            tools: None },
    ]
}
```

> **Clone helper:** add a small `fn clone_refs(&self) -> EvalDaemonRefs` (or derive the daemon over `Arc` fields and `#[derive(Clone)]`) so spawned tasks own cheap clones. Implement it by cloning the `Arc`s and `Copy`/`Clone` scalars. The simplest path is to make all fields `Arc`/`Copy` and `#[derive(Clone)]` the struct, then call `self.clone()` in `poll_once`.

- [ ] **Step 4: Export + build**

In `crates/arkavo-orchestrator/src/lib.rs` add `pub use eval_daemon::EvalDaemon; mod eval_daemon;` (match the file's existing export style).

Run: `cargo build -p arkavo-orchestrator --features ... 2>&1 | tail -30`
Expected: builds once `list_prs`/`OrgDiscovery`/`GitHubPullRequest` field names match Step 1. Fix mismatches (e.g. `pr.head_sha` vs `pr.head.sha`).

- [ ] **Step 5: Unit-test the contract builder (pure)**

Add a `#[cfg(test)]` test in `eval_daemon.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_has_three_preconditions_and_prompts() {
        let c = build_contract("gemma-4-12b", "b3:0", "deadbeef");
        assert_eq!(c.preconditions.len(), 3);
        assert!(c.prompts.iter().any(|p| p.id == "tool_call"));
        assert_eq!(c.acceptance.min_similarity, 0.87);
    }
}
```

Run: `cargo nextest run -p arkavo-orchestrator contract_has_three`
Expected: PASS.

- [ ] **Step 6: clippy + commit**

Run: `cargo clippy -p arkavo-orchestrator -- -D warnings`

```bash
git add crates/arkavo-orchestrator/src/eval_daemon.rs crates/arkavo-orchestrator/src/lib.rs crates/arkavo-orchestrator/Cargo.toml
git commit -m "arkavo-orchestrator: org-wide EvalDaemon"
```

---

## Phase 7 — CLI daemon entrypoint + gossip pointer (optional)

### Task 10: `arkavo eval daemon` subcommand

**Files:**
- Modify: `crates/arkavo-cli/src/commands/eval.rs`
- Modify: `crates/arkavo-cli/src/lib.rs`

- [ ] **Step 1: Add a `daemon` runner to `eval.rs`**

```rust
pub fn daemon(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut org: Option<String> = None;
    let mut interval = 300u64;
    let mut max_concurrent = 10usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--org" => { i += 1; org = args.get(i).cloned(); }
            "--interval" => { i += 1; interval = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(300); }
            "--max-concurrent" => { i += 1; max_concurrent = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(10); }
            other => return Err(format!("unknown eval daemon arg: {other}").into()),
        }
        i += 1;
    }
    let org = org.ok_or("missing --org <org>")?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        // Wire GitHubApp from env (GITHUB_APP_ID + key, or GITHUB_TOKEN), the
        // OrchestratorStateStore, MemoryEmbedder, and the production TDF+iroh
        // baseline store (see Part 2 Task 5). resolve_model maps a model name to
        // (gguf_path, b3 weight digest) from the HF cache + a committed digest map.
        // Then:
        //   let daemon = EvalDaemon::new(org, app, baselines, embedder, state,
        //       Duration::from_secs(interval), max_concurrent, resolve_model);
        //   daemon.run().await;
        let _ = (org, interval, max_concurrent); // replace with wiring above
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
```

> This step's body is wiring, not new logic — assemble the components built in earlier tasks. Keep credential loading consistent with how `arkavo-orchestrator`/`arkavo-github` already read `GITHUB_APP_ID`/`GITHUB_TOKEN` (grep for those env names to match the existing convention).

- [ ] **Step 2: Dispatch `eval daemon`**

In `crates/arkavo-cli/src/lib.rs`, extend the `"eval"` arm from Part 1:

```rust
        "eval" => match args.get(1).map(|s| s.as_str()) {
            Some("run") => commands::eval::run(&args[2..]),
            Some("daemon") => commands::eval::daemon(&args[2..]),
            _ => Err("usage: arkavo eval <run|daemon> ...".into()),
        },
```

- [ ] **Step 3: Build + commit**

Run: `cargo build -q && cargo clippy -p arkavo-cli -- -D warnings`

```bash
git add crates/arkavo-cli/src/commands/eval.rs crates/arkavo-cli/src/lib.rs
git commit -m "arkavo-cli: 'eval daemon' org-wide entrypoint"
```

### Task 11 (optional): Broadcast baseline pointers over gossip

**Files:**
- Modify: `crates/arkavo-eval/src/historian_tdf.rs` or `eval_daemon.rs`

- [ ] **Step 1: After `publish()` succeeds, broadcast the `BaselinePointer`**

`arkavo-gossip`'s `GossipProtocol` carries enum `GossipMessage` variants. Add a `BaselinePointerAnnounce(BaselinePointer)` variant (or reuse a generic announce), then in the daemon after a `main` publish, call the protocol's `handle_message`/broadcast path. This makes pointers discoverable by other agents without a fetch. Defer if the swarm is single-node — the local index already serves fetches.

- [ ] **Step 2: Commit (if implemented)**

```bash
git add -A && git commit -m "arkavo-eval: broadcast baseline pointers over gossip"
```

---

## Phase 8 — Final verification (Part 2)

### Task 12: Full build, lint, pre-push checks

- [ ] **Step 1: Build with all relevant features**

Run: `cargo build -q -p arkavo-eval --features "embeddings llama-cpp tdf-iroh"`
Expected: success on the macOS swarm member (llama-cpp builds non-musl).

- [ ] **Step 2: Tests (offline subset)**

Run: `cargo nextest run -p arkavo-eval --features "embeddings tdf-iroh" && cargo nextest run -p arkavo-github -p arkavo-memory -p arkavo-orchestrator`
Expected: PASS (the llama live test stays `#[ignore]`).

- [ ] **Step 3: No-OpenSSL guard**

Run the repo's no-openssl check (see `.github/workflows/feature.yaml` `no-openssl-check`) against a musl build to confirm the daemon's TDF/iroh features didn't pull OpenSSL. Note: the daemon binary that enables `opentdf`/`kas`/`llama-cpp` targets macOS, not musl; ensure the musl build path excludes these features.

- [ ] **Step 4: fmt + clippy**

Run: `cargo fmt -- --check && cargo clippy --workspace -- -D warnings`
Expected: clean (or scope clippy to touched crates if the full workspace is noisy on unrelated code).

- [ ] **Step 5: Live end-to-end (manual, on the swarm member with models cached)**

```bash
# With GITHUB_APP_ID/key (or GITHUB_TOKEN) exported and Gemma weights cached:
cargo run -p arkavo --features ... -- eval daemon --org arkavo-org --interval 300 --max-concurrent 10
```
Expected: opens PRs touching model paths get `arkavo-eval/gemma-4-12b` and `arkavo-eval/gemma-4-E2B` checks; merging to main records baselines; subsequent PRs compare against them.

---

## Self-review notes (author)

- The real Operator, baseline store, and daemon each land behind a feature/trait introduced in Part 1, so Part 1 stays green independently.
- `weights_attested` is a *real* BLAKE3 verify of the resident GGUF — valuable even though weights aren't iroh-distributed yet.
- Baselines are genuinely TDF-encrypted + iroh-staged + b3-addressed + commit-keyed; the round-trip is proven offline with a mock cipher and in-memory iroh, and the production OpenTDF+KAS wiring is documented with exact constructors (Task 5).
- Known follow-ups (flagged, not silently dropped): merge-base baseline resolution (currently anchored to `main`), gossip pointer broadcast for multi-node discovery (Task 11, optional), the nightly 5-model sweep (E2B/E4B/12B/26B/31B by schedule or label), folding PR discovery into `OrgPoller` proper, abstracting the trigger behind a `Poll`/`Webhook` source trait, the Scribe per-PR plaintext record artifact, and the Operator's `arkavo-attestation` evidence capture (recorded-not-gated).
```

