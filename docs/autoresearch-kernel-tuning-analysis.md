# Autoresearch × Arkavo Edge: In-Depth Feasibility Analysis

Grounded assessment of how Karpathy's autoresearch pattern maps onto the Arkavo Edge codebase, based on reading the actual source files, benchmarks, specs, and architecture.

## What Autoresearch Is

Karpathy's autoresearch is a ~630-line, three-file repo where a human writes a `program.md` to instruct an AI agent that autonomously iterates on `train.py` — modifying architecture, hyperparameters, optimizer settings — while `prepare.py` remains fixed as the evaluation harness. The core loop: read own code → form hypothesis → modify → run 5-minute experiment → evaluate → keep or revert → repeat.

After ~700 autonomous changes over two days, it found ~20 additive improvements that dropped "Time to GPT-2" from 2.02h to 1.80h (11% efficiency gain) on already well-tuned code.

## Integration Point Analysis

### Metal Kernel Auto-Tuning

**Status: Infrastructure exists, kernel is real, tuning surface is well-defined**

The DeltaNet Metal kernel exists as patches applied to vendored llama.cpp:

- `crates/arkavo-llama-cpp-sys/patches/003-deltanet-metal-kernel.patch` — 106-line MSL kernel implementing `kernel_delta_net_f32()` with GDA/KDA gate modes
- `crates/arkavo-llama-cpp-sys/patches/004-deltanet-metal-dispatch.patch` — Metal dispatch with `ggml_metal_op_delta_net()` configuration

**Kernel architecture:**
- Row-per-thread design: thread `tid` owns row `tid` of the SxS state matrix
- Grid: `(B*H, 1, 1)` threadgroups with `(C/H, 1, 1)` threads per threadgroup
- Shared memory: 128-element `threadgroup float` arrays for k, q vectors
- SIMD optimization: `float4` vectorized inner loops with `dot()` intrinsics
- Gate dispatch: compile-time `G==1` branch for GDA vs runtime KDA
- Head size constraint: max 128 (MSL compile-time array size requirement)
- Supported head sizes: 64 or 128 (asserted in dispatch: `C/H == 64 || C/H == 128`)

**Tunable parameters within the kernel:**

| Parameter | Current Value | Search Space | Location |
|---|---|---|---|
| Threadgroup dims | `(C/H, 1, 1)` | `(32,1,1)` to `(128,1,1)` | `004-deltanet-metal-dispatch.patch:108` |
| SIMD unroll factor | 4 (float4) | 2, 4, 8 | `003-deltanet-metal-kernel.patch:80-88` |
| Shared memory size | 128 | 64, 128, 256 | `003-deltanet-metal-kernel.patch:47-48` |
| Gate branch strategy | Runtime `G==1` check | Separate kernels vs unified | `003-deltanet-metal-kernel.patch:71-73` |
| Decay clamp | `min(g_raw, 88.0f)` | 50.0–100.0 | `003-deltanet-metal-kernel.patch:74` |

**Tunable parameters outside the kernel (inference-level):**

| Parameter | Current Value | Search Space | Location |
|---|---|---|---|
| Batch chunk size | `[16, 32, 64, 128]` | 8–512 | `performance.rs:153` |
| Sampler temperature | 0.1–0.7 by model | 0.0–2.0 | `llamacpp_provider.rs:700-717` |
| Top-p | 0.9 | 0.5–1.0 | hardcoded in benchmarks |
| Top-k | 40 | 10–100 | hardcoded in benchmarks |

**Evaluation harness (prepare.py equivalent):**

The benchmark suite is production-ready:
- `crates/arkavo-llm/benches/deltanet_throughput.rs` — measures tok/s, TTFT, prompt eval, token eval for Qwen3.5 via `perf_context()` metrics
- `crates/arkavo-llama-cpp/benches/performance.rs` — context creation, tokenization, batch processing, single token generation, TTFT across prompt lengths
- Criterion configuration: 20-30s measurement time, 10-50 samples, 3s warm-up

**Metric:** `perf_context().tok_per_sec()` — already implemented, returns tokens/second including both prompt evaluation and generation.

**The autoresearch loop would be:**
1. **Fixed:** Criterion benchmark harness + `perf_context()` metrics
2. **Mutable:** Kernel parameters (requires patch regeneration + rebuild) OR inference parameters (runtime config)
3. **Budget:** 5-minute wall-clock per experiment (maps to Criterion's `measurement_time`)
4. **Keep/revert:** Compare tok/s against baseline, commit winning config

**Challenge:** Kernel-level tuning requires modifying the `.patch` file and rebuilding llama.cpp (`cargo build -p arkavo-llama-cpp-sys`). This is a ~2-minute rebuild cycle, leaving ~3 minutes for actual measurement within a 5-minute budget. Inference-level parameter tuning is instant — no rebuild required.

**Recommendation:** Start with inference-parameter sweeps (zero rebuild cost), then graduate to kernel parameters once the loop is validated.

**Feasibility: High**

---

### Thinking Gate Optimization

**Status: Binary gate exists at a known location, ready for continuous sweep**

The gate at `llamacpp_provider.rs:345-346`:
```rust
enable_thinking = !(format == ModelFormat::Qwen3 && is_small_model(&self.name))
```

Where `is_small_model()` (lines 68-78) matches: `0.6b`, `0.8b`, `270m`, `500m`.

**What autoresearch would optimize:**

| Dimension | Current | Search Space |
|---|---|---|
| Size boundary | Binary: sub-1B = off | Continuous sweep 0.5B–3B |
| Temperature coupling | Independent (0.1–0.7) | Joint optimization with thinking gate |
| Thinking depth | All-or-nothing | Partial: inject `<think>` with max-token cap |
| Template injection | Post-template, model-specific | Prompt template search |

**Evaluation harness:** `CriticPipeline::verify()` at `crates/arkavo-critic/src/pipeline.rs:79-173` — priority-ordered checks returning `PipelineResult { passed, evidence, total_latency_us }`. Already measures quality with microsecond precision.

**Composite metric:** `quality_per_token = critic_pass_rate / total_tokens_generated`. This rewards both accuracy and efficiency — a thinking model that uses 3x tokens for marginal quality gains would score lower.

**Feasibility: High** — smallest mutable surface (1 boolean + 1 float), production evaluation harness exists.

---

### Agent Context Management

**Status: Full infrastructure, strategies are the tunable surface**

The `Conductor` at `crates/arkavo-hrm/src/conductor/orchestrator.rs` implements:

- `ContextStrategy` enum: `ArtifactReference`, `Ledger` (offloading to `MemoryStorage`)
- `min_offload_chars` threshold: default 2000 (~500 tokens), configurable via `with_min_offload_threshold()`
- Context summarization: optional LLM-powered summary generation before offloading
- Budget tracking: `TaskBudget` with cost, tokens, duration, spending rate

**What autoresearch would optimize:**

| Parameter | Default | Range |
|---|---|---|
| `min_offload_chars` | 2000 | 500–10000 |
| Strategy selection | Manual | Bandit over strategies |
| Summarization prompt | Hardcoded | Template search |
| Burst limits | `max_steps`, `max_wall_time`, `max_tokens`, `max_cost_usd` | Joint sweep |

**Evaluation signal:** The `LoopDetector` at `crates/arkavo-hrm/src/conductor/loop_detector.rs` already tracks success/failure per task description with Jaccard similarity (0.85 threshold). Natural "did context strategy lead to completion?" signal.

**Challenge:** Requires a held-out conversation corpus with known-good completions, or the Critic pipeline pass rate as proxy. The metric is fuzzier than tok/s.

**Feasibility: Medium-high** — infrastructure solid, metric definition needs work.

---

### Distributed Autoresearch via Gossip

**Status: Protocol is production-ready, experiment coordination layer is missing**

The gossip protocol at `crates/arkavo-gossip/src/protocol.rs`:

- Epidemic propagation: configurable fanout (default 3), O(log n) convergence
- Anti-entropy: 30-second intervals with digest comparison
- Quorum consensus: 2/3 threshold via `ConsensusState`
- Ed25519 verification: full signing chain at `verification.rs`
- Message types: `PatchAnnouncement`, `PatchVote`, `PatchDelivery`, `LessonAnnouncement`, `ContextManifestAnnouncement`

The Thompson Sampling at `crates/arkavo-router/src/learning/agent_utility.rs`:

```rust
pub struct BetaPrior { alpha: f64, beta: f64 }  // Beta(2,1) cold start
// Quality-weighted: success with quality 0.9 adds 0.9 to alpha (not binary +1)
// Exploration floor: 5% minimum selection probability
// Per-category priors: HashMap<String, BetaPrior>
// Windowed priors for concept drift adaptation
```

**Mapping:**

| Autoresearch Concept | Gossip Primitive | Status |
|---|---|---|
| Run experiment | `PatchAnnouncement` | Exists |
| Report result | `LessonAnnouncement` | Exists |
| Keep or discard | `PatchVote` with quorum | Exists |
| Propagate winner | `PatchDelivery` | Exists |
| Allocate experiments | `BetaPrior::sample()` Thompson Sampling | Exists |
| Experiment logging | `DecisionTrace` with `SelectionReason` | Exists |

**Missing:** An `ExperimentCoordinator` (~200 lines) wrapping gossip messages with experiment-specific semantics — configuration vectors instead of code diffs, metric comparison instead of code review.

**Challenge:** Experiment reproducibility across heterogeneous hardware (M4 Max vs. Pi 5). Different hardware produces different absolute tok/s, so the metric must account for cross-hardware comparability.

**Metric design — relative improvement is necessary but insufficient.** A naive relative-improvement metric (% over node-local baseline) introduces a subtle bias: Thompson Sampling will over-explore the Pi 5 because it appears to have more headroom (15% improvement easy on a weak baseline) while under-exploiting the M4 Max (5% improvement on an already-optimized baseline is far more valuable in absolute terms). The quality weight fed to `BetaPrior::apply_fractional_update()` should incorporate absolute performance:

```
quality = relative_improvement × log(baseline_tok_per_sec)
```

This biases toward improvements on already-fast nodes. A 5% gain on a 200 tok/s M4 Max baseline (quality = 0.05 × log(200) = 0.265) outweighs a 15% gain on a 10 tok/s Pi 5 baseline (quality = 0.15 × log(10) = 0.150). The log compression prevents extreme nodes from dominating — it's a gentle preference, not a hard cutoff.

This formula slots directly into the existing `BetaPrior` quality-weighted update path, where success with quality 0.9 adds 0.9 to alpha rather than binary +1.

**Feasibility: Medium** — all primitives exist, need a thin coordination layer + the weighted metric.

---

### TØR-G Policy Circuit Evolution

**Status: Evaluation is sub-microsecond, but mutation operators don't exist**

Circuit benchmarks at `crates/arkavo-critic/benches/circuit_eval.rs` confirm sub-μs evaluation. The `arkavo-torg` crate implements constrained decoding via `TorgLlamaSampler` with model-specific token mappings (`Qwen3TokenMap`, `MinistralTokenMap`).

Preflight moderation at `crates/arkavo-router/benches/preflight.rs` runs at < 5ms for production policy sets (PII, SQL injection, shell commands, base64).

**The problem:** Circuit evolution is genetic programming, not parameter sweeping. Mutating boolean circuit topology while maintaining semantic validity requires designing mutation operators (add gate, remove gate, swap feature inputs, change gate type). This is qualitatively harder than sweeping continuous parameters.

**The 139ns evaluation time is already so fast** that speed optimization is pointless — the value would be in *coverage* optimization (catch more bad inputs without more false positives). The metric (F1 × inverse-latency) conflates two dimensions where one is already saturated.

**Recommendation:** Defer topology mutation until simpler optimizations are validated. However, there is a sweepable surface within existing circuits: the preflight moderator's pattern-matching confidence thresholds are continuous parameters. These thresholds (PII detection sensitivity, SQL injection confidence cutoffs, base64 entropy thresholds) live within fixed circuit topologies and are amenable to autoresearch-style parameter sweeping without any mutation operators. The evaluation metric is F1 over a held-out adversarial prompt set — a well-defined, fast evaluation. This would be Phase 5+ work, after Phases 1-3 validate the autoresearch loop on simpler surfaces.

**Feasibility: Low-medium** (topology mutation) / **Medium** (threshold sweeping within fixed topology)

---

### The program.md ↔ Agent Role System

**Status: Strong conceptual alignment, validated by code**

The actual role implementations:

| Role | Implementation | Key File |
|---|---|---|
| Architect | Task decomposition, model selection, cost estimation | `crates/arkavo-router/src/architect/{planner,executor,complexity}.rs` |
| Critic | Priority-ordered verification, evidence collection | `crates/arkavo-critic/src/pipeline.rs` |
| Conductor | Task lifecycle, budget, loop detection, context | `crates/arkavo-hrm/src/conductor/orchestrator.rs` |
| Orchestrator | GitHub integration, cognitive planning | `crates/arkavo-orchestrator/src/orchestrator.rs` |

The mapping validated against actual code:

| Autoresearch | Arkavo Edge (Actual Implementation) |
|---|---|
| `program.md` | 62 spec files in `specs/arkavo-edge/` + 74 `AGENTS.md` files |
| `train.py` (mutable) | Architect's planner prompts + Router's model selection |
| `prepare.py` (fixed) | `CriticPipeline::verify()` — priority-ordered, fail-fast, μs tracking |
| `val_bpb` | `PipelineResult { passed, evidence, total_latency_us }` |
| Git branch/experiment | Gossip `PatchAnnouncement` with SHA-256 deduplication |
| Keep/revert | `PatchVote` with 2/3 quorum consensus |
| Loop prevention | `LoopDetector` — max 3 failures, 0.85 Jaccard similarity |
| Experiment allocation | `BetaPrior::sample()` Thompson Sampling |

## The Core Insight

Arkavo Edge already implements every component of the autoresearch feedback loop — it just doesn't close it automatically:

1. **Hypothesis formation** — Architect's `ComplexityScorer` + `ArchitectPlanner`
2. **Bounded execution** — Conductor's `BurstContract` with `max_wall_time` budget
3. **Evaluation** — Critic's `CriticPipeline` with evidence collection
4. **Learning** — Thompson Sampling's `BetaPrior` with quality-weighted updates
5. **Propagation** — Gossip protocol with quorum consensus
6. **Loop prevention** — `LoopDetector` with thrashing detection

The gap is closing the loop — and the right place to close it is inside the Conductor, not as a standalone binary.

### AutoResearch as a Specialized BurstContract

The `Conductor` already manages `BurstContract` instances with `max_wall_time`, `max_steps`, `max_tokens`, and `max_cost_usd` budgets. The `LoopDetector` already prevents thrashing via Jaccard similarity (0.85 threshold) and failure counting (max 3). An autoresearch experiment is a `BurstContract` where the "task" is "improve this metric."

The runner (~300 lines) extends `Conductor` with an `AutoResearchBurst` that implements the seven-step loop:

1. **Read config** — load experiment parameters from TOML (the `program.md` equivalent)
2. **Mutate** — apply one parameter change from the search space
3. **Execute** — call into `perf_context()` or Criterion benchmark harness
4. **Evaluate** — compare against baseline via `PipelineResult` or raw tok/s
5. **Decide** — keep/revert using a threshold (no quorum needed for single-node Phases 1-3)
6. **Log + Commit** — append to `results.tsv`, commit winning config to a git branch
7. **Loop** — repeat until `BurstContract.max_wall_time` exhausted

Step 6 is worth highlighting: borrowing Karpathy's git-branch-per-experiment pattern gives a full audit trail. Each winning configuration gets committed to a branch, enabling `git bisect` if a "winning" config degrades on a different workload. The gossip protocol's `PatchAnnouncement` with SHA-256 deduplication already speaks this language — you create the local git commit, then announce it. Losing configs get reverted, not committed.

This design reuses `BurstContract` for budget enforcement, `LoopDetector` for thrashing prevention, and `TaskStore` for persistence across crashes — all of which already exist and are tested.

## Priority-Ordered Implementation Plan

### Phase 1: Thinking Gate Sweep (1-2 days)

Create `crates/arkavo-llm/tests/thinking_gate_sweep.rs`:
- Load Qwen3.5-0.8B via `LlamaModel::from_file()`
- Fixed prompt set extracted from existing CLI tests
- Sweep: `enable_thinking` × `temperature` × `max_thinking_tokens`
- Measure quality via `CriticPipeline::verify()`, speed via `perf_context().tok_per_sec()`
- Report `quality_per_token` as composite metric
- Keep/revert via writing winning config to a TOML file

### Phase 2: Unified Parameter Sweep — Inference + Kernel (3-5 days)

Extend `deltanet_throughput.rs` into an `AutoResearchBurst` within the Conductor:

**Inference parameters (zero rebuild):**
- Batch chunk size, sampler temperature, top-p, top-k, context window
- Objective: `perf_context().tok_per_sec()`
- Keep/revert: run baseline, run variant, compare, commit winning config to branch
- 5-minute `BurstContract.max_wall_time` per experiment

**Kernel parameters (zero rebuild via precompiled variants):**
- Precompile N kernel variants at build time with distinct MSL function names
- Sweep threadgroup dimensions: `(32,1,1)`, `(64,1,1)`, `(128,1,1)`
- Sweep unroll factors: float2 vs float4 vs float8 inner loops
- Test GDA-only kernel variant (eliminate runtime `G==1` branch)
- Runtime dispatch selects variant via `kernel_variant` op parameter
- Same 5-minute budget, same evaluation harness as inference sweeps

**Joint search:** Temperature × thinking_gate × kernel_variant form a combined search space. Thompson Sampling's per-category priors (`HashMap<String, BetaPrior>`) naturally partition this into independent dimensions that can be explored concurrently.

### Phase 3: Distributed Experiment Coordination (1-2 weeks)

Add `ExperimentMessage` to gossip protocol:
- Configuration vector (not code diff) as payload
- Metric result as `LessonAnnouncement` response with `baseline_tok_per_sec` field
- Thompson Sampling allocates configs to nodes by hardware capability
- Weighted quality metric: `relative_improvement × log(baseline_tok_per_sec)`
- `BetaPrior::apply_fractional_update(quality)` consumes the weighted score directly
- Git commit per winning config, announced via `PatchAnnouncement`

### Phase 4: Context Strategy Optimization (2-3 weeks)

Build conversation evaluation corpus, then:
- Sweep `min_offload_chars`, context strategy selection
- Use `LoopDetector` success rate as evaluation signal
- Feed winning strategies into `Conductor::with_context_strategy()`

## Key Risk: The Rebuild Bottleneck

Kernel-level tuning faces a constraint autoresearch doesn't: the mutable file is a C++ patch applied during `cargo build`, not a Python script. Each experiment requires a full rebuild of `arkavo-llama-cpp-sys`. On the M4 Max with ccache, this is ~2 minutes — consuming 40% of a 5-minute budget.

### The Precompiled Variant Solution (Eliminates the Bottleneck)

The dispatch code already selects between kernel configurations at runtime. In `004-deltanet-metal-dispatch.patch`, `ggml_metal_library_get_pipeline_delta_net()` asserts `C/H == 64 || C/H == 128` and does a pipeline lookup by name string:

```cpp
const char * name = "kernel_delta_net_f32";
ggml_metal_pipeline_with_params res = ggml_metal_library_get_pipeline(lib, name);
if (!res.pipeline) {
    res = ggml_metal_library_compile_pipeline(lib, name, name, nullptr);
}
```

This pattern extends naturally to N precompiled kernel variants. Each variant gets a distinct MSL function name (e.g., `kernel_delta_net_f32_tg32_unroll4`, `kernel_delta_net_f32_tg64_unroll8`) compiled once at build time. The dispatch function selects among them via a configuration parameter — identical to how it already dispatches on head_size.

**This collapses kernel tuning into the same zero-rebuild infrastructure as inference parameter sweeps.** The threadgroup dimensions, unroll factors, and gate branch strategies all become runtime-selectable. The tradeoff is binary size (~2KB per variant × maybe 12 variants = ~24KB) and one-time compile cost — negligible on M4 Max.

The dispatch modification is ~30 lines: a `switch` on a new `kernel_variant` field in `ggml_op_params`, mapping to pre-registered pipeline names. The kernel patch grows by N × (kernel size), but since each variant is a mechanical substitution (different `float4` → `float2` unrolls, different shared memory sizes), it's template-like duplication.

**Result:** Kernel tuning experiments run at the same speed as inference parameter experiments — zero rebuild, immediate measurement. This merges Phase 3 into Phase 2's autoresearch infrastructure.

### Remaining Mitigations (for novel kernel architectures)

For truly novel kernel modifications that can't be precompiled (e.g., adding a new inner loop structure), the rebuild bottleneck remains:
- **Extend budget**: 10-minute windows for structural experiments (~6 experiments/hour)
- **Parallel builds**: Build next variant while benchmarking current one (two worktrees)
- **Inference-first validation**: Confirm the autoresearch loop works on zero-rebuild parameters before investing in structural changes

## Metrics Summary

| Phase | Integration Point | Primary Metric | Secondary Metric | Target |
|---|---|---|---|---|
| 1 | Thinking gate | quality_per_token | CriticPipeline pass rate | Find optimal boundary |
| 2 | Inference + kernel params | tok/s via `perf_context()` | TTFT (ms) | >15% improvement |
| 3 | Distributed experiments | weighted quality × nodes | winning config transfer rate | 12×N experiments/hour |
| 4 | Context management | task completion rate | time-to-completion | >10% completion improvement |
| 5+ | Policy circuit thresholds | F1 score | evaluation latency (ns) | Maintain <1μs, improve F1 |
