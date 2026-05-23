# NGRAM Self-Speculative Decoding Design

Date: 2026-05-23
Status: Approved, ready for implementation
Author: Paul Flynn (with Claude)
Companion to: PR #596 (`feature/llama-cpp-b9292`), commits to stack on top

## Goal

Reduce tool-loop second-inference (Infer2) latency by wiring b9292's `common_speculative` with `COMMON_SPECULATIVE_TYPE_NGRAM_SIMPLE` into the local llama.cpp generation loop. No extra GGUF, no per-model configuration required.

## Non-goals

- MTP (`COMMON_SPECULATIVE_TYPE_DRAFT_MTP`) — needs per-model capability detection and a separate `LLAMA_CONTEXT_TYPE_MTP` context. Deferred to a follow-up once NGRAM ships.
- Draft model (`COMMON_SPECULATIVE_TYPE_DRAFT_SIMPLE`) — needs config surface, second GGUF load, tokenizer-compat checks. Deferred.
- N-gram cache persistence across requests. Would amplify gains for repetitive workloads. Deferred.
- Remote provider paths (Kimi, Gemini, Anthropic). Out of scope; those have their own spec mechanisms upstream.

## Motivation

Memory note `feedback_infer2_bottleneck` describes Infer2 in our tool loop running 2-5× slower than Infer1. After tool execution, the model consumes a structured tool result (typically JSON or a table) and continues generation. Tool results have high repetition — recurring JSON keys, list separators, identifier substrings — which is the canonical strong case for n-gram self-speculative decoding.

NGRAM_SIMPLE keeps a rolling n-gram cache of the prompt + emitted tokens and predicts the continuation when the recent n-gram has been seen. When the prediction is correct, multiple tokens are committed in a single decode pass. When the prediction is wrong, the cost is one extra batch slot — near-zero overhead.

We use only a small slice of b9292's new surface today (FlashAttention type setting, `llama_perf_context`, common_chat parsing). NGRAM spec is the next-highest-leverage item that doesn't require code we don't already have.

## Architecture

Single integration point in `crates/arkavo-llm/src/llamacpp_streaming.rs`. The tool loop in `conductor_tool_loop.rs` doesn't change — it calls into the provider and gets faster transparently.

| Layer | New surface |
|---|---|
| `arkavo-llama-cpp-sys` | `arkavo_spec_wrapper.{cpp,h}` — extern "C" wrapper around `common_speculative_*` (init/begin/process/draft/accept/free). Compiled into the existing `arkavo_chat_wrapper` lib via the existing `cc_build` block in `build.rs`. |
| `arkavo-llama-cpp` | `SpeculativeContext` newtype with `new(model, n_seq) -> Result`, `begin(seq, prompt)`, `draft() -> Vec<token>`, `accept(seq, n)`, `Drop`. ~80 lines. |
| `arkavo-llm` | Generation loop in `llamacpp_streaming.rs` wraps decode with `begin → loop { draft → batch_eval → walk accept-prefix → emit → accept(n) }`. Reads `use_spec_decoding` from `CompletionOptions`; emits raw `n_draft` / `n_accepted` in `InferenceTiming`. No thresholds, no env reads, no policy. ~100-150 line diff. |
| `CompletionOptions` (`arkavo-llm`) | Gains `use_spec_decoding: bool`. Passed from the router into the provider per request. |
| `arkavo-router` | Owns policy. `RoutingDecision` gains `use_spec_decoding: bool` next to the existing `should_compress`. Router maintains per-model rolling accept-rate stats — sibling to the existing `model_learning` and `AntiPattern` infrastructure — and decides the flag from those stats. Telemetry feedback path consumes `n_draft` / `n_accepted` from completed requests to update the stats. Emits a structured router event when a model drops below the accept-rate threshold; no log lines (per `feedback_telemetry_not_logging`). |
| `tool_bench` | Reads router's per-model stats to print `accept_rate` and `spec_speedup_pct` per model alongside existing Infer1/Infer2 columns. |

## Data flow

Today, the generation loop is:

```
batch = [next_token]
decode(batch)
sample
emit
repeat
```

With spec:

```
spec.begin(prompt)
loop:
    draft = spec.draft()                  # n_draft tokens predicted by n-gram cache
    batch = [next_token, ...draft]        # submit all at once
    decode(batch)                         # verify in parallel
    walk logits: accept while sampled == drafted
    emit accepted tokens
    spec.accept(seq, n_accepted)
    next_token = first divergent sampled token
```

When the n-gram cache has no useful prediction, `draft()` returns empty, the batch shrinks to one token, and we degenerate to baseline.

## Per-model variability

NGRAM helps or hurts depending on workload and model:

- **Wins**: medium/large models (≥3B) on structured output. High accept rate (40-70%); draft eval cheap relative to per-token gen.
- **Loses**: small/fast models (0.8B-3B) where draft batch eval overhead ≥ savings, or high-entropy creative gen where accept rate approaches zero.

The fast-synthesis path uses `qwen3.5-0.8b` (per `MEMORY.md` "Multi-Model Agent Architecture") — exactly the case where NGRAM may hurt. Without per-model visibility, we'd silently slow that path down.

All three policy mechanisms live in `arkavo-router`. The streaming layer is a dumb executor — it does spec when the routing decision tells it to and reports the numbers.

1. **Per-model accept-rate stats** (router). Rolling window per-model keyed by model identity — GGUF hash if PR #595's `ModelAttestor` content-addressing has merged by implementation time, otherwise the loaded model name string from `llama_model_meta_val_str` (`general.name`). The watch-and-warn loop only needs a stable per-model key, not the cryptographic guarantee. Updated from `n_draft` / `n_accepted` in `InferenceTiming` via the post-execution feedback path that already flows back into router learning.

2. **Per-request policy decision** (router). When constructing a `RoutingDecision`, the router consults the per-model accept-rate stats. If the rolling average is below the threshold (proposed: 15% over 20 requests), `use_spec_decoding = false` for that model. Otherwise true. Insufficient data ⇒ true (try it, gather data, let the router decide later). This is the **escape hatch** — no env var, no manual list. The router auto-detects per CLAUDE.md philosophy ("Auto-detect capabilities. No manual configuration.").

3. **Structured warning event** (router). When a model crosses below threshold, the router emits a structured event onto its existing telemetry stream (e.g., `RouterEvent::SpecDecodingDisabled { model, accept_rate, sample_size }`) for the UI to surface, instead of a log line. Consistent with `feedback_telemetry_not_logging` ("Use telemetry aggregated in UI, not log lines").

## Risk register

| Risk | Mitigation |
|---|---|
| Sampling parity break (spec produces different tokens than baseline) | Parity test: 100-token completions with `ARKAVO_SPEC_NGRAM=0` vs `=1` on a deterministic seed; assert identical token IDs. Fails CI if they diverge. |
| KV cache misalignment on rollback | Use `common_speculative_accept(spec, seq, n)` exclusively — never roll back positions ourselves. Test paths: 0 accepted, all accepted, partial accepted. |
| Grammar interaction (tool calls must still be grammar-constrained) | Speculative sampling integrates with the existing sampler chain. Tool-call test must still produce valid tool calls; existing tests in `arkavo-llm` cover this. |
| Silent regression on small models | Router-owned per-model accept-rate stats + auto-skip below threshold (see Per-model variability). The streaming layer can't silently regress because it has no policy of its own — it does what the router says, and the router downgrades it as soon as the data shows the model is a bad fit. |
| Hot path complexity making future debugging harder | Speculative context owned by the provider, lifetime tied to context. Generation loop change is contained to one function. Comment documents the invariant. |

## Verification (before claiming done)

- [ ] New parity test: identical output with/without spec at fixed seed, multiple prompts
- [ ] `cargo test -p arkavo-llm` — all 200+ pass
- [ ] `cargo test --workspace --exclude arkavo` — green
- [ ] `cargo clippy -- -D warnings` — green
- [ ] `tool_bench` reports `accept_rate >= 30%` on the 9B planner path on a tool-loop scenario (the case we're optimizing for)
- [ ] `tool_bench` shows neutral-or-better Infer2 ms on at least one workload
- [ ] Router auto-skips spec for a forced-low-accept-rate model (test that drives accept rate to 0 for one model and verifies subsequent `RoutingDecision.use_spec_decoding == false` for that model while a high-accept model stays true)
- [ ] `RouterEvent::SpecDecodingDisabled` emitted exactly once per model crossing the threshold (no flapping, no log noise)

## Non-goals (also)

- Removing `use_spec_decoding` from the router's auto-decision in favor of an env var or per-model AGENTS.md flag. Auto-detection from rolling stats is the design; manual overrides would erode the "no manual configuration" property.
- Persisting per-model accept-rate stats across process restarts. In-memory rolling window is enough for v1; persistence can be added later if the warm-up cost is measurably significant.

## Planned commits (stacked on `feature/llama-cpp-b9292`)

1. `arkavo_spec_wrapper: expose common_speculative as C API` — `.cpp` + `.h`, build.rs hookup
2. `arkavo-llama-cpp: safe SpeculativeContext wrapper` — Rust safe surface + unit tests
3. `arkavo-llm: integrate NGRAM spec decoding into streaming generation` — generation loop change, `CompletionOptions.use_spec_decoding` field, raw `n_draft`/`n_accepted` in `InferenceTiming`, parity test. No policy.
4. `arkavo-router: per-model spec accept-rate stats and auto-skip` — rolling window in `model_learning` (or sibling), `RoutingDecision.use_spec_decoding`, threshold check, `RouterEvent::SpecDecodingDisabled` structured event
5. `tool_bench: report per-model spec accept-rate and spec_speedup_pct`

Effort estimate: ~400-500 line diff across 6 files. 4-6 hours focused. Most risk in commit 3 (generation loop change); commit 4 is bounded by the existing `model_learning` patterns.
