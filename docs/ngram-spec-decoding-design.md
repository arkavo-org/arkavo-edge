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
- Auto-disable based on per-model performance history. Ship visibility first; auto-disable lands in a follow-up once we have real-workload data.

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
| `arkavo-llm` | Generation loop in `llamacpp_streaming.rs` wraps decode with `begin → loop { draft → batch_eval → walk accept-prefix → emit → accept(n) }`. ~100-150 line diff. |
| Config | `ARKAVO_SPEC_NGRAM` env (default `1` = on, set `0` to disable globally). `ARKAVO_SPEC_NGRAM_DISABLE` comma-separated substrings to skip per model. Read once at provider construction. |
| Telemetry | Extend `InferenceTiming` with `n_draft: Option<u32>`, `n_accepted: Option<u32>`. Per-model rolling map kept in the provider for the watch-and-warn loop. `tool_bench` aggregates and prints per-model `accept_rate` and `spec_speedup_pct`. |

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

Three mechanisms:

1. **Per-model telemetry.** `n_draft`, `n_accepted`, `accept_rate`, `spec_speedup_pct` keyed by model identity. Use the model GGUF hash if PR #595's `ModelAttestor` content-addressing has merged by implementation time; otherwise fall back to the loaded model name string from `llama_model_meta_val_str` (`general.name`). The watch-and-warn loop only needs a stable per-model key, not the cryptographic guarantee. Surfaced in `InferenceTiming` and aggregated in `tool_bench` output.

2. **Watch-and-warn.** Rolling 20-request average accept rate per model. If under 15%, emit a one-time WARN log:
   ```
   spec decoding accept rate 8% for model qwen3.5-0.8b over last 20 requests
   — consider ARKAVO_SPEC_NGRAM_DISABLE=qwen3.5-0.8b
   ```
   No auto-disable in this PR. Land visibility first.

3. **Escape hatch.** `ARKAVO_SPEC_NGRAM_DISABLE=qwen3.5-0.8b,phi-3-mini` — substring match against model name, matching models skip spec for the session.

## Risk register

| Risk | Mitigation |
|---|---|
| Sampling parity break (spec produces different tokens than baseline) | Parity test: 100-token completions with `ARKAVO_SPEC_NGRAM=0` vs `=1` on a deterministic seed; assert identical token IDs. Fails CI if they diverge. |
| KV cache misalignment on rollback | Use `common_speculative_accept(spec, seq, n)` exclusively — never roll back positions ourselves. Test paths: 0 accepted, all accepted, partial accepted. |
| Grammar interaction (tool calls must still be grammar-constrained) | Speculative sampling integrates with the existing sampler chain. Tool-call test must still produce valid tool calls; existing tests in `arkavo-llm` cover this. |
| Silent regression on small models | Per-model telemetry + watch-and-warn (see above). |
| Hot path complexity making future debugging harder | Speculative context owned by the provider, lifetime tied to context. Generation loop change is contained to one function. Comment documents the invariant. |

## Verification (before claiming done)

- [ ] New parity test: identical output with/without spec at fixed seed, multiple prompts
- [ ] `cargo test -p arkavo-llm` — all 200+ pass
- [ ] `cargo test --workspace --exclude arkavo` — green
- [ ] `cargo clippy -- -D warnings` — green
- [ ] `tool_bench` reports `accept_rate >= 30%` on the 9B planner path on a tool-loop scenario (the case we're optimizing for)
- [ ] `tool_bench` shows neutral-or-better Infer2 ms on at least one workload
- [ ] Watch-and-warn fires (does not crash) when forced into low-accept conditions

## Planned commits (stacked on `feature/llama-cpp-b9292`)

1. `arkavo_spec_wrapper: expose common_speculative as C API` — `.cpp` + `.h`, build.rs hookup
2. `arkavo-llama-cpp: safe SpeculativeContext wrapper` — Rust safe surface + unit tests
3. `arkavo-llm: integrate NGRAM spec decoding into streaming generation` — generation loop change, parity test, env gate
4. `arkavo-llm: per-model spec telemetry and watch-and-warn` — per-model rolling accept rate, warn log, disable env
5. `tool_bench: report spec accept-rate and spec_speedup_pct per model`

Effort estimate: ~400-500 line diff across 6 files. 4-6 hours focused. Most risk in commit 3 (generation loop change).
