# Local-Model Eval Gate — Design

- Date: 2026-06-13
- Status: Draft for review
- Scope: First slice of the Arkavo Edge GitHub agent-swarm — running CI tests that depend on local models.

## Context and goal

Arkavo Edge will run an agent swarm under a dedicated GitHub identity that monitors the
org's repos and performs housekeeping, extended CI, and PR reviews. The first concrete task
is to run the deterministic local-model eval tests (the `gemma4_*` suites) as a PR gate.

GitHub-hosted runners cannot host 7–48 GB GGUF weights, so these tests are `#[ignore]`'d
today and never run in CI. The unlock is an Arkavo Edge swarm member on Apple-Silicon
hardware (Metal) that already has the weights cached. The swarm runs the eval "in the
optimal way for the current state of the swarm" and reports a verdict back to GitHub as a
required check.

This slice favors a working vertical slice: real evals gating real PRs, with the five-role
governance structure honest but its heavier guarantees (iroh distribution, AIA/SPIRE
attestation, TDF-encrypted records, distributed scheduling) stubbed behind clean trait seams
for later increments.

## What exists today

- `arkavo-swarmkit` + `arkavo-swarmkit-runtime`: manifest parsing, `SwarmFlight::launch()`,
  `dispatch_initial_tasks()`. Roles are manifest-driven with free-form `role_type`; the
  five-role mesh (Planner/Operator/Critic/Scribe/Historian) is a recommended vocabulary, not
  a hardcoded mesh. No `TaskKind` enum and no `submit()`/`wait_terminal()` lifecycle yet.
- `arkavo-tdf-iroh`: iroh blob transport with BLAKE3 tickets (`BlobTransport::stage_bytes/fetch_bytes`).
- `arkavo-swarmkit/src/canonical.rs`: `content_hash()` emits `blake3:<base64url>` — not the
  `b3:<hex>` form the contract schema uses.
- `arkavo-torg`: TØR-G boolean policy graphs that `evaluate(inputs) -> bool` — the real engine
  behind `policy_circuit: "torg:<id>"`.
- `arkavo-llama-cpp` + `arkavo-llm::LlamaCppProvider`: vendored llama.cpp FFI, GPU
  detect/fallback, Gemma-4 grammar patch.
- `arkavo-memory::EmbeddingService`: bundled offline AllMiniLML6V2 ONNX model (fastembed),
  `generate_embedding()` + `cosine_similarity()`, 384-dim, deterministic CPU inference (gated by
  the `embeddings` feature).
- `arkavo-github`: App/PAT auth, `OrgPoller` (org-capable: discovery interval, max-concurrent
  repos, include/exclude, archived/label filters), `GitHubOperations` (PRs/issues/comments/labels/
  releases). No check-run or commit-status methods.
- `arkavo-orchestrator`: Axum webhook server (HMAC), GitHub App auth.
- `arkavo-attestation`: macOS Secure-Enclave evidence collection (info only, no signing) +
  software-fingerprint fallback. No SPIRE/AIA.
- `arkavo-llm/tests/gemma4_12b_test.rs`, `gemma4_compare_test.rs`: `#[ignore]`'d, load weights
  from the HF cache, produce a capability scorecard + load_s/infer_s.

## What this slice does not build (deferred behind trait seams)

- iroh weight fetch-by-digest — `WeightSource` trait; slice impl reads the HF cache.
- AIA/SPIRE attestation and C2PA provenance enforcement — `ProvenanceVerifier` trait; slice
  impl returns `not-enforced` (logged), records evidence without gating on it.
- TDF-encrypted eval record — `RecordSink` trait; slice impl writes plaintext JSON to a local
  content-addressed store.
- Distributed SwarmKit task submission / placement — slice executes a single-node `SwarmFlight`
  (honors REQ-6.4: single-node/local swarm capability).

## Architecture

A long-lived Arkavo Edge swarm process runs on the Apple-Silicon Mac in `eval-daemon` mode.
It polls the org over outbound HTTPS only (no inbound reachability required), and for each PR
that touches model-behavior code it runs the five-role eval pipeline as a single-node
`SwarmFlight` and posts a GitHub Check Run as the gate.

```
arkavo-orchestrator (eval-daemon on the Mac swarm member)
  │  OrgPoller — org discovery + bounded concurrency + per-repo circuit breaker (outbound HTTPS)
  ▼
EvalHandler  ──eligible PR? (path/label filter, idempotency via eval-state)──▶ Check Run "queued"
  │
  ▼  builds Eval Task Contract for <repo, head_sha, model>
arkavo-eval :: single-node SwarmFlight
  Planner   resolve contract → EvalPlan (models, prompt-set, baseline digest, exec profile)
  Critic⊕   TØR-G pre-flight gate (torg_core::Graph.evaluate) → allow | TYPED REFUSAL
  Operator  LlamaCppProvider load (HF cache) → run prompt-set @ seed=0/temp=0 → outputs + tok_s
  Critic⊖   EmbeddingService cosine-sim vs Historian baseline + tok_s → regression verdict
  Scribe    write b3-addressed eval record via RecordSink (plaintext JSON now; TDF later)
  Historian local baseline store keyed by b3:<hex>; records new baseline on merge-to-main
  │
  ▼  terminal TypedStatus
EvalHandler ──update Check Run──▶ success | failure | neutral | action_required + annotations
```

### Trigger choice

Polling is the default trigger because the Mac swarm member sits behind NAT with no stable
inbound route; webhooks would need a tunnel/relay (an operational burden). Polling needs only
outbound HTTPS. The trigger is abstracted behind a source trait (`Poll` now, `Webhook`/`Action`
later) so other deployments drop in without touching the eval pipeline.

## Crate and module layout

New crate `arkavo-eval` (each module under 400 lines, `std`-first, rustls-only, musl-safe):

- `contract.rs` — Eval Task Contract type + (de)serialization.
- `digest.rs` — `b3:<hex>` helpers and verification; conversion to/from existing `blake3:` form.
- `plan.rs` — Planner: contract → `EvalPlan`.
- `gate.rs` — Critic pre-flight gate; builds a `torg_core::Graph` from preconditions.
- `operator.rs` — Operator: model load + prompt-set execution; `WeightSource` + `ProvenanceVerifier` traits.
- `verdict.rs` — Critic post-flight: semantic similarity + tok_s acceptance → verdict.
- `record.rs` — Scribe: eval record type + `RecordSink` trait (plaintext content-addressed sink).
- `historian.rs` — baseline store (local, b3-keyed) + baseline lifecycle.
- `status.rs` — typed status taxonomy.
- `lib.rs` — `run_eval(contract) -> TypedStatus` over a single-node `SwarmFlight`.

Touched existing crates:

- `arkavo-github`: add `create_check_run` / `update_check_run` to `GitHubOperations`
  (REST `POST/PATCH /repos/{owner}/{repo}/check-runs`, existing App-token auth).
- `arkavo-orchestrator`: add `EvalHandler` + an `eval-daemon` mode wired to `OrgPoller`;
  add a SQLite-backed eval-state store (idempotency + restart survival), following existing
  `arkavo-memory` storage patterns.

Reused as-is: `arkavo-llm` (`LlamaCppProvider`), `arkavo-memory` (`EmbeddingService`),
`arkavo-torg` (`Graph`), `arkavo-swarmkit[-runtime]` (single-node `SwarmFlight` + eval-kit
manifest), `arkavo-attestation` (evidence).

## The five roles

### Planner

Resolves the committed Eval Task Contract for the PR's git SHA into an `EvalPlan`: the model
set, the prompt-set, the baseline digest, and the pinned execution profile
(`seed=0, temp=0` greedy, fixed `threads`/`ctx`). Model selection reflects "optimal for current
swarm state": the per-PR plan targets the default install model (12B) plus the smallest variant
(E2B), downshifting if a variant is not resident.

### Critic — pre-flight (TØR-G gate)

Builds a boolean policy `Graph` from the contract's preconditions and `evaluate()`s it.
Enforced now: `weights_present` and `weights_attested` are a real BLAKE3 verify of the resident
GGUF against `weight_digest` (valuable even without iroh), and `baseline_present`. Deferred
(return `not-enforced`, logged): `provenance_valid` (C2PA), AIA/SPIRE attestation. On an unmet
precondition with `on_precondition_unmet: refuse`, emit a typed refusal.

### Operator

Loads the model via `LlamaCppProvider` from the HF cache (iroh fetch-by-digest deferred behind
`WeightSource`), runs the prompt-set under the pinned profile, captures outputs + tokens/sec.
Collects `arkavo-attestation` evidence into the record (recorded, not gated).

### Critic — post-flight (verdict)

Embeds each output and its baseline reference via `EmbeddingService`, computes cosine
similarity. Acceptance: aggregate similarity ≥ 0.87 AND tok_s ≥ 95% of baseline. Emits a
regression verdict. Deterministic ONNX embedding makes the verdict reproducible even though the
gemma4 output is not bit-exact on Metal — this is why semantic similarity is used rather than
exact-output match.

### Scribe

Writes the eval record (contract, digests, attestation evidence, per-prompt metrics, verdict)
as a `b3:<hex>`-addressed artifact via the `RecordSink` trait. Slice impl: plaintext JSON to a
local content-addressed store. TDF-encrypted sink is a later trait impl.

### Historian

Supplies the prior baseline for `baseline.digest` from the local store (seeded from a committed
baseline file so it is reproducible from a git SHA per REQ-5.2). Records a new baseline when the
eval runs on `main` after merge.

## Eval Task Contract

A Rust struct mirroring the doc schema (`task_kind: model_eval`), committed to the repo and
content-addressed (REQ-5.1/5.2). It references models/baselines by `b3:<hex>` digest, never a
mutable bucket key. Acceptance metrics for this slice: `semantic_similarity >= 0.87` and
`tok_s >= 0.95` (relative to baseline). `policy_circuit: "torg:<id>"` references a committed
circuit definition. This is a distinct artifact from the existing `arkavo-protocol`
`task_contract.rs` (an operator↔critic negotiation, unrelated to model eval).

`digest.rs` adds the `b3:<hex>` form the schema uses; the existing `content_hash()`
(`blake3:<base64url>`) is left intact for its current callers.

## Typed status taxonomy

| TypedStatus | Check Run conclusion | Meaning |
|---|---|---|
| `Passed` | `success` | acceptance met |
| `RegressionFailed { metric, value, threshold }` | `failure` | similarity or tok_s below threshold |
| `Refused { reason }` | `action_required` | pre-flight gate denied (digest mismatch, baseline absent+required, …) |
| `BaselineBootstrapped` | `neutral` | first run; nothing to compare, record becomes the candidate baseline |
| `InfraError { stage }` | `failure` (distinct annotation) | model load / swarm error — explicitly not a model regression |
| `Skipped` | no check posted | PR does not touch model paths |

`InfraError` must be visibly distinct from `RegressionFailed` so an infrastructure failure is
never mistaken for a model quality regression.

One Check Run is posted per `(PR head, model)` with a stable context name `arkavo-eval/<model>`
(e.g. `arkavo-eval/gemma-4-12b`, `arkavo-eval/gemma-4-E2B`), so branch protection can require
each model's check independently and a regression on one model does not mask the other.

## Trigger and eligibility

- Org-wide from the start via `arkavo-github::OrgPoller` (discovery + bounded concurrency +
  per-repo circuit breaker for error isolation).
- Eligibility (default, overridable): a PR is eligible when it touches `crates/arkavo-llm/`,
  `crates/arkavo-llama-cpp{,-sys}/`, `vendor/llama.cpp/`, `crates/arkavo-router/src/decision.rs`,
  `crates/arkavo-torg/`, or the contract/baseline files; label overrides `eval:local-models`
  (force) and `eval:skip` (suppress).
- Idempotency + restart survival: a SQLite-backed eval-state store keyed by
  `(repo, head_sha, model)` records the `TypedStatus` and `check_run_id`. The handler does not
  re-run a completed eval; on a new head SHA it runs again and updates the check.

## Baseline lifecycle

Per-PR runs compare against the baseline blessed on `main`. A new model or new prompt yields
`BaselineBootstrapped` (neutral, not a failure). Baselines are auto-recorded when the eval runs
on `main` after merge — promotion is the merge event, not a per-PR mutation.

## Per-PR vs nightly model selection

- Per-PR gate: default model (12B) + smallest variant (E2B) prompt-sets.
- Nightly / on-demand: full 5-model `gemma4_compare_test` sweep (E2B, E4B, 12B, 26B-A4B, 31B),
  triggered by schedule or label, not on every PR.

## TØR-G pre-flight gate

The gate is expressed as a boolean policy `Graph` (existing `torg_core`) whose inputs are the
precondition booleans. The slice builds the circuit programmatically from the contract's
preconditions and `evaluate()`s it; LLM-constrained-decoding generation of policy from natural
language is out of scope here. `policy_circuit` references a committed circuit definition.

## Determinism and execution profile

Eval runs use `seed=0, temp=0` (greedy) with pinned `threads`/`ctx` to minimize variance. Metal
is not bit-exact run-to-run, which is accepted: the verdict is semantic-similarity-based and the
embedding model is deterministic.

## Security and constraints

- rustls only, no OpenSSL; musl-safe; `cargo fmt` + `cargo clippy -D warnings` clean; no
  `#[allow(dead_code)]`.
- Eval records may contain model outputs; the local record sink stays on the swarm member.
  Existing DLP/PII security tests must continue to pass.
- No hardcoded paths; weights located via the existing HF-cache discovery.

## Testing strategy

- TDD; ≥85% coverage; every bug fix gets a regression test.
- Roles are unit-tested with a fake Operator feeding known outputs; the deterministic
  `EmbeddingService` lets us assert exact verdicts against thresholds.
- Integration test drives contract → (faked) outputs + baseline → asserts each `TypedStatus`
  (Passed / RegressionFailed / Refused / BaselineBootstrapped / InfraError).
- `create_check_run` / `update_check_run` tested against a mocked GitHub REST endpoint.
- Real gemma4 runs remain `#[ignore]`'d (require local weights).

## Operational setup (documented, partly outside code)

- A dedicated GitHub identity (machine user or GitHub App installation) for Arkavo Edge with
  permissions to read PRs and write Check Runs across the org.
- The Arkavo Edge swarm process runs `eval-daemon` on the Apple-Silicon Mac with the Gemma-4
  weights cached and the `embeddings` feature enabled.
- Branch protection makes the eval check context required (a one-time org/repo settings step).

## Acceptance criteria for this slice

- A PR touching model-behavior code in a monitored repo produces a GitHub Check Run whose
  conclusion reflects the typed verdict.
- The verdict is reproducible across repeated runs of the same head SHA (deterministic
  embedding verdict), and infra errors are distinguishable from regressions.
- Merging to `main` records the new baseline; subsequent PRs compare against it.
- The pipeline runs as a single-node `SwarmFlight` on the Mac swarm member with weights from the
  HF cache; iroh/attestation/TDF seams exist but are not required for the gate to function.

## Open risks

- First-run baseline bootstrapping across many repos org-wide could generate noise; mitigated by
  the path/label eligibility filter and the `neutral` bootstrap status.
- Embedding-similarity thresholds need empirical tuning; 0.87 is the starting point inherited
  from the contract schema and should be calibrated against real baseline runs.
- Per-PR latency for 12B + E2B on a single Mac must stay within reviewers' tolerance; the
  nightly sweep absorbs the heavier variants.
