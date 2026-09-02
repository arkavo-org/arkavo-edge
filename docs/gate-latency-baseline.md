# Dispatch Gate Latency Baseline

Baseline for Epic 0 item 2: per-stage p50/p95 on the policy+sequence path, to
judge whether a dispatch gate can meet a 25ms end-to-end budget.

## Methodology

- Benchmark: `crates/arkavo-router/benches/gate_latency.rs` (criterion).
- Reproduce: `cargo bench -p arkavo-router --bench gate_latency`
- Machine: Apple M4 Max (arm64), 2026-08-26.
- Build: criterion bench profile (optimized). Debug-build numbers are not
  meaningful for a latency budget and are not reported.
- p50/p95: 200 manual samples per stage after a 20-iteration warmup, printed
  as `GATE_STAGE` lines. Mean: criterion's statistical estimate.

## Measured stages

| Stage | p50 | p95 | Criterion mean |
|---|---|---|---|
| Preflight moderator, 3 policies (clean input) | 1.67µs | 1.71µs | 1.27µs |
| Budget check (`BudgetTracker::can_afford`) | 0.08µs | 0.12µs | 72.7ns |
| Critic CircuitCheck (1 policy registered) | 1.62µs | 1.83µs | 1.49µs |
| Critic SchemaCheck (1 tool call) | 0.92µs | 0.96µs | 0.74µs |
| Critic PolicyCheck (security defaults) | 0.46µs | 0.46µs | 0.40µs |
| Critic default pipeline (Circuit+Schema+Policy) | 3.71µs | 4.38µs | 3.30µs |
| Ed25519 signature verify | 18.08µs | 18.62µs | 17.71µs |
| **Full policy+sequence path** (preflight + budget + critic pipeline) | **5.25µs** | **5.38µs** | **4.40µs** |

The full path combines the three stages a dispatch gate would run in sequence:
preflight moderation on the task text, a budget affordability check, and the
default critic pipeline over a response carrying one tool call.

## What the 25ms budget can and cannot contain

The measured local path consumes roughly **0.02% of the 25ms budget** — over
three orders of magnitude of headroom. Comfortably inside the budget:

- All critic fast checks: CircuitCheck (~1.5µs), SchemaCheck (~0.7µs),
  PolicyCheck (~0.4µs), and LintCheck (same regex/string-scan class, <2ms per
  crate docs).
- Preflight moderation with a handful of TØR-G policies (~1.3µs for 3).
- Budget checks and reservations (~0.1µs; `try_spend` additionally appends a
  history record and fires a broadcast event — same in-memory class).
- Signature verification: Ed25519 verify measured at 17.7µs; P-256 ECDSA
  verify is the same sub-millisecond class (~50–100µs in pure Rust). Dozens of
  verifies fit inside the budget.

What blows the 25ms budget, and therefore cannot be on the gate's critical
path:

- **Any network hop.** One internet RTT is tens of milliseconds; even a LAN
  round trip is ~0.5ms. KAS/TDF key fetches, remote attestation, or cloud
  policy lookups must be cached or pre-fetched before the gate runs.
- **SemanticCheck** (local-model coherence judge): ~50ms target by design —
  2x the entire gate budget. It can only run post-dispatch or off-path.
- **Local LLM inference of any kind**, including tiny classifiers.
- **Uncached disk/IPC round trips** at contention. Individual ones are
  sub-millisecond, but they stack; policy and circuit artifacts should be
  loaded at startup.

## Runtime observability

A `dispatch_gate` rolling-window tracker now lives in
`SubsystemTimingRegistry` (`crates/arkavo-observability/src/subsystem_timing.rs`)
and is recorded at three hook sites:

- Preflight moderation in `Router::route` (`crates/arkavo-router/src/lib.rs`).
- Budget reservation in `RouterOrchestrator` (`crates/arkavo-router/src/orchestrator.rs`).
- Critic verification in `verify_response_with_critic`
  (`crates/arkavo-cli/src/tool_integration.rs`).

Caveat: the registry records whole milliseconds, so these sub-millisecond
stages currently read as 0 in the `dispatchGateAvgMs`/`dispatchGateP95Ms`
snapshot fields. The registry answers "did gate overhead regress to whole
milliseconds?"; this bench provides the sub-ms precision. If per-stage ms
resolution ever matters, switch the tracker samples to microseconds.
