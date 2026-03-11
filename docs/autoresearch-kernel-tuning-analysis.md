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

**Challenge:** Experiment reproducibility across heterogeneous hardware (M4 Max vs. Pi 5). Different hardware produces different absolute tok/s, so the metric must be *relative improvement* (% change from that node's baseline).

**Feasibility: Medium** — all primitives exist, need a thin coordination layer.

---

### TØR-G Policy Circuit Evolution

**Status: Evaluation is sub-microsecond, but mutation operators don't exist**

Circuit benchmarks at `crates/arkavo-critic/benches/circuit_eval.rs` confirm sub-μs evaluation. The `arkavo-torg` crate implements constrained decoding via `TorgLlamaSampler` with model-specific token mappings (`Qwen3TokenMap`, `MinistralTokenMap`).

Preflight moderation at `crates/arkavo-router/benches/preflight.rs` runs at < 5ms for production policy sets (PII, SQL injection, shell commands, base64).

**The problem:** Circuit evolution is genetic programming, not parameter sweeping. Mutating boolean circuit topology while maintaining semantic validity requires designing mutation operators (add gate, remove gate, swap feature inputs, change gate type). This is qualitatively harder than sweeping continuous parameters.

**The 139ns evaluation time is already so fast** that speed optimization is pointless — the value would be in *coverage* optimization (catch more bad inputs without more false positives). The metric (F1 × inverse-latency) conflates two dimensions where one is already saturated.

**Recommendation:** Defer until simpler optimizations are validated. If pursued, constrain the search space: vary only feature weights and thresholds within existing circuit topologies, don't add/remove gates.

**Feasibility: Low-medium**

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

The gap is a ~300-line `AutoResearchRunner` that wires these into the read → hypothesize → modify → run → evaluate → keep/revert cycle.

## Priority-Ordered Implementation Plan

### Phase 1: Thinking Gate Sweep (1-2 days)

Create `crates/arkavo-llm/tests/thinking_gate_sweep.rs`:
- Load Qwen3.5-0.8B via `LlamaModel::from_file()`
- Fixed prompt set extracted from existing CLI tests
- Sweep: `enable_thinking` × `temperature` × `max_thinking_tokens`
- Measure quality via `CriticPipeline::verify()`, speed via `perf_context().tok_per_sec()`
- Report `quality_per_token` as composite metric
- Keep/revert via writing winning config to a TOML file

### Phase 2: Inference Parameter Sweep (2-3 days)

Extend `deltanet_throughput.rs` into an autoresearch loop:
- Parameterize: batch chunk size, sampler temperature, top-p, top-k, context window
- Objective: `perf_context().tok_per_sec()`
- Keep/revert: run baseline, run variant, compare, commit winning config
- 5-minute wall-clock budget per experiment

### Phase 3: Kernel Parameter Tuning (1 week)

Modify `003-deltanet-metal-kernel.patch` programmatically:
- Sweep threadgroup dimensions: `(32,1,1)`, `(64,1,1)`, `(128,1,1)`
- Sweep unroll factors: float2 vs float4 vs float8 inner loops
- Test GDA-only kernel (eliminate runtime `G==1` branch)
- Each experiment: modify patch → rebuild llama-cpp-sys → run benchmark → compare
- ~2 min rebuild + ~3 min measurement = fits 5-minute budget

### Phase 4: Distributed Experiment Coordination (1-2 weeks)

Add `ExperimentMessage` to gossip protocol:
- Configuration vector (not code diff) as payload
- Metric result as `LessonAnnouncement` response
- Thompson Sampling allocates configs to nodes by hardware capability
- Relative improvement metric (% over node-local baseline) for cross-hardware comparison

### Phase 5: Context Strategy Optimization (2-3 weeks)

Build conversation evaluation corpus, then:
- Sweep `min_offload_chars`, context strategy selection
- Use `LoopDetector` success rate as evaluation signal
- Feed winning strategies into `Conductor::with_context_strategy()`

## Key Risk: The Rebuild Bottleneck

Kernel-level tuning (Phase 3) faces a constraint autoresearch doesn't: the mutable file is a C++ patch applied during `cargo build`, not a Python script. Each experiment requires a full rebuild of `arkavo-llama-cpp-sys`. On the M4 Max with ccache, this is ~2 minutes — consuming 40% of the 5-minute budget.

Mitigation strategies:
- **Inference-first** (Phase 2): Sweep parameters that don't require rebuild
- **Precompiled variants**: Generate N kernel variants at build time, select at runtime via function pointers
- **Extend budget**: Use 10-minute windows for kernel experiments (still ~6 experiments/hour)
- **Parallel builds**: Build next variant while benchmarking current one (requires two worktrees)

## Metrics Summary

| Integration Point | Primary Metric | Secondary Metric | Target |
|---|---|---|---|
| Metal kernel tuning | tok/s via `perf_context()` | TTFT (ms) | >15% improvement |
| Thinking gate | quality_per_token | CriticPipeline pass rate | Find optimal boundary |
| Context management | task completion rate | time-to-completion | >10% completion improvement |
| Distributed experiments | experiments/hour × nodes | winning config transfer rate | 12×N experiments/hour |
| Policy circuits | F1 score | evaluation latency (ns) | Maintain <1μs, improve F1 |
