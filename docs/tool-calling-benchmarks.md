# Tool-Calling Benchmarks Across Local Models

Benchmarked using `arkavo tool-bench` with 8 standardized scenarios: single-param, multi-param, no-param, enum, file path, command execution, should-not-call, and multi-type params. Five test tools registered (get_weather, read_file, search, get_time, run_command).

## Results (2026-04-04)

| Model | Size | Parse | Tool Name | Params | Avg Latency |
|-------|------|-------|-----------|--------|-------------|
| **Qwen3.5-0.8B** | **0.8B** | **8/8** | **8/8** | **8/8** | **525ms** |
| **Ministral-3-3B** | **3B** | **8/8** | **8/8** | **8/8** | **690ms** |
| Ministral-3-8B | 8B | 8/8 | 8/8 | 8/8 | 1,409ms |
| Qwen3.5-9B | 9B | 8/8 | 8/8 | 8/8 | 1,905ms |
| Gemma-4-E2B | 2.3B (5.1B w/ PLE) | 8/8 | 8/8 | 8/8 | 2,229ms |
| GLM-4.7-Flash | 4.7B (30B MoE) | 8/8 | 8/8 | 8/8 | 2,698ms |
| Gemma-4-E4B | 4.5B (8B w/ PLE) | 1/8 | 1/8 | 1/8 | 1,298ms |
| Gemma-4-26B-A4B | 4B active (26B MoE) | 8/8 | 8/8 | 8/8 | 7,410ms |
| Qwen3.5-27B | 27B | 7/8 | 7/8 | 7/8 | 41,634ms |

## Key Findings

**Gemma-4-E2B achieves perfect 8/8 at 2,276ms.** The smallest Gemma 4 variant (2.3B active params, 5.1B total with PLE) uses llama.cpp's native Gemma 4 Jinja template with `<|tool_call>` format. All 8 scenarios pass via the text-extraction fallback chain. Slower than Ministral-3B but notable as the first Gemma family model to achieve 8/8 tool calling.

**Gemma-4-E4B scores 1/8 — needs non-lazy grammar sampler integration.** The E4B (4.5B active) Jinja template produces a non-lazy GBNF grammar (`grammar_lazy=false`) with generation_prompt prefill (`<|turn>model\n`). llama-server handles this via its full sampler pipeline (grammar init → prefill with generation_prompt tokens → constrained generation), but our standalone grammar sampler cannot replicate this state management — the grammar stack underflows when encountering `<|tool_call>` transitions. Without grammar-constrained generation, E4B generates text responses instead of tool calls. The native PEG output parser (PR #21418) is wired up and ready — the blocker is grammar-constrained generation only.

**Qwen3.5-0.8B fixed: 1/8 → 8/8.** Previously broken due to missing parser support for Qwen's native `<parameter=key>value</parameter>` format. With the named-parameter parser and production text-extraction fallbacks now wired into the bench, all 8 scenarios pass at 525ms average.

**All models up to 9B achieve perfect 8/8** (except Gemma-4-E4B). Qwen3.5-0.8B, Ministral-3B, Ministral-8B, Qwen3.5-9B, Gemma-4-E2B, and GLM-4.7-Flash all produce correct tool calls.

**Ministral-3-3B remains the recommended default.** Perfect 8/8 at 690ms — the best latency/accuracy tradeoff for 3B+ models. Qwen3.5-0.8B is faster (525ms) but uses the native `<tool_call>` format requiring text-extraction fallback rather than direct fence parsing.

**GLM-4.7-Flash is slower than Ministral-8B** (2,698ms vs 1,409ms) despite MoE architecture. The 30B total parameter count negates MoE efficiency on memory-bandwidth-bound Apple Silicon. Requires 32GB RAM.

**Qwen3.5-27B degrades under load.** 7/8 accuracy with one scenario (command_execution) taking 269s — likely hitting generation length limits or think-block runaway. Suitable for batch/offline tasks only.

### Bench Uses Production Code Path

The bench now wires through the same post-processing as the production router:
1. `LlamaCppProvider::complete_with_tools()` — inference with Jinja template + tool-calling temperature
2. `filter_and_extract_tool_calls()` — language fence filtering
3. `extract_tool_calls_from_text()` — fallback chain: fence → JSON → XML → Python-style → curly-brace
4. Model discovery via `ModelChoice` registry + `is_model_cached()`

## Improvements Implemented

### Qwen Named-Parameter Parser

`parse_function_eq_format()` now handles Qwen3.5's `<parameter=key>value</parameter>` format via `extract_named_parameters()`. Previously only `<parameter>JSON</parameter>` (single tag with JSON body) was supported. Values are type-inferred: `true`/`false` → boolean, digits → number, everything else → string. This fixed Qwen3.5-0.8B from 1/8 → 8/8 and Qwen3.5-9B from 6/8 → 8/8.

### Refined Retry Feedback
`ValidationError::fix_suggestion()` generates specific, actionable error messages when tool calls fail validation. Instead of generic "include ALL required parameters", the model receives:
- Exact missing parameter name and example syntax
- List of available tools on hallucinated tool errors
- Expected type with example values on type mismatches

### Few-Shot Prompt Injection
`example_generator.rs` produces schema-aware example values using type inference, enum values, defaults, and name-based heuristics (e.g., `file_path` gets `/path/to/file`). Replaces static placeholder examples in fence-format prompts.

### Tool Schema Distillation
`to_fence_prompt_distilled()` offers three detail tiers for token-constrained models:
- **NameOnly**: ~200 chars — tool names + generic format instruction
- **NameAndDescription**: ~540 chars — names, descriptions, required param names, one example
- **FullSchema**: ~700 chars — full parameter schemas with types

### Model-Specific Temperature Tuning
Tool-calling inference uses lower temperatures for smaller models:
- Sub-1B: 0.1 (near-greedy)
- 3B: 0.2
- GLM-4: 0.15
- 8B+: default

### GBNF Grammar (Opt-In)
`tool_grammar.rs` generates GBNF grammars constraining output to valid fence-format tool calls. Supports lazy grammar patterns (trigger on `` ``` ``) for free text before tool blocks. Currently opt-in due to crashes with some model/quantization combinations (Ministral-3B Q4_K_M).

### Search Tools Meta-Tool
`SearchToolsTool` in the registry allows the LLM to discover tools mid-conversation by keyword, useful when initial tool extraction misses relevant tools.

## Learning Mesh Validation

Tested with a 4-agent learning mesh (orchestrator + code-analyzer + test-generator + security-auditor) using 11 registered tools (`shell_exec`, `git_diff`, `code_review`, `list_agents`, `get_task_status`, `filesystem_tools`, `test_run`, `git_status`, `git_log`, `agent_query`, `send_task`).

### Tool Calling in Multi-Agent Context

- **Fence-format parsing works reliably** across orchestrator and specialist agents. Tools like `filesystem_tools`, `send_task`, `code_review`, and `list_agents` are called with correct syntax.
- **Multi-step tool loops function correctly.** The orchestrator chains tool calls across iterations (e.g., `list_agents` → `send_task` to delegate to a specialist).
- **Error learning propagates via gossip.** When a tool call fails (e.g., `code_review` with missing git context), the error is captured as a correction and broadcast to other agents via the gossip protocol. Subsequent attempts benefit from injected guidance.
- **Thompson Sampling model cooling works.** Ministral-8B was cooled down for 3600s on code-analyzer after 8 consecutive quality failures (inference timeouts), allowing other models to take over.
- **Advisor adjustments flow between agents.** `ToolError` and `NoToolCalls` dynamic adjustments from specialists are received and applied by the orchestrator, improving prompt advice for future tasks.

### Observed Issues

- **Path resolution**: Models sometimes use relative paths (`/crates/...`) instead of absolute paths, causing `filesystem_tools` failures. This is a prompt engineering issue, not a parsing issue.
- **Inference timeouts on complex tasks**: 8B models can timeout (30s limit) on tasks with large tool result context, triggering quality cooldown.
- **Quality score 0.0 on tool-only responses**: When the model responds with only a tool call and no text, the quality scorer assigns 0.0. The tool call itself may succeed — this is a scoring gap, not a tool calling failure.

## Chat Path Tool Calling

The `--prompt` chat path now supports full tool calling via `route_chat()` → `provider.complete_with_tools()`. Previously, tools were listed in a text system message but never wired to the provider, causing hallucinated responses.

### Model Override

```bash
# Test specific model
arkavo chat --model ministral-3b --prompt "What time is it?"
arkavo chat --model glm-4.7-flash --prompt "What time is it?"
```

### Debug Telemetry

With `ARKAVO_DEBUG=1`, metadata deltas render inline diagnostics:

```
[Model] ministral-3b (Chat path: fastest local model, separate semaphore)
[Tools] keywords="time" found=5 tools=[get_time, get_weather, ...]
[Perf] 422ms | 47 chars | 1 tool calls | prompt_eval: 89ms gen: 333ms | 142.3 tok/s
```

## Raw Inference Performance

Baseline throughput via `llama-bench` (pp512 prompt processing, tg128 text generation). Apple Silicon unified memory. These numbers represent the theoretical ceiling before tool-calling overhead (prompt construction, parsing, validation).

| Model | Size | Quant | PP (t/s) | TG (t/s) |
|-------|------|-------|----------|----------|
| Qwen3-0.6B | 0.6B | Q8_0 | 9,872 | 293 |
| Qwen3.5-0.8B | 0.8B | Q4_K_M | 5,861 | 170 |
| Ministral-3-3B | 3B | Q4_K_M | 1,900 | 136 |
| GLM-4.7-Flash | 4.7B (MoE 30B) | Q4_K_M | 1,153 | 75 |
| Ministral-3-8B | 8B | Q5_K_M | 765 | 55 |
| Qwen3.5-9B | 9B | Q4_K_M | 767 | 50 |
| Qwen3.5-27B | 27B | Q6_K_XL | 226 | 14 |

## llama-server Tool Calling (Native)

Baseline tool-calling accuracy via `llama-server` with `tool_bench.py` (temp=0, n=8). Tests native llama-server tool calling without Arkavo's fence-format layer.

| Model | hello_world | weather | Avg Latency |
|-------|-------------|---------|-------------|
| Qwen3-0.6B | 8/8 | 8/8 | 622ms |
| Qwen3.5-0.8B | 8/8 | 0/8* | 500ms |
| **Ministral-3-3B** | **8/8** | **8/8** | **192ms** |
| GLM-4.7-Flash | 8/8 | 8/8 | 1,090ms |
| Ministral-3-8B | 8/8 | 8/8 | 456ms |
| Qwen3.5-9B | 8/8 | 8/8 | 1,032ms |
| Qwen3.5-27B | 8/8 | 8/8 | 6,411ms |

*Qwen3.5-0.8B weather failure: refuses to call the tool, responds with text instead.

**Ministral-3B dominates at 192ms** — 3x faster than the next-best (Ministral-8B at 456ms) with perfect accuracy on both scenarios. This confirms the choice as default chat model.

**Qwen3.5-0.8B passes hello_world but fails weather.** The model can produce tool call syntax for trivial cases but refuses parameterized calls, generating text responses instead.

## Cross-Benchmark Analysis

Comparing native llama-server (`tool_bench.py`, 2 scenarios) against Arkavo Edge (`tool-bench`, 8 scenarios) reveals the overhead of the full tool-calling pipeline.

| Model | llama-server | Arkavo Edge | Ratio | Where Time Goes |
|-------|-------------|-------------|-------|-----------------|
| Qwen3.5-0.8B | 500ms | 525ms | 1.05x | Minimal overhead — native `<tool_call>` format parsed via text-extraction fallback |
| Ministral-3-3B | 192ms | 690ms | 3.6x | Pipeline overhead dominates (Jinja template, prompt construction, fence parsing, validation) |
| Ministral-3-8B | 456ms | 1,409ms | 3.1x | Similar pipeline overhead, slightly amortized by slower inference |
| Qwen3.5-9B | 1,032ms | 1,905ms | 1.8x | Native `<parameter=key>` format, think-block overhead |
| GLM-4.7-Flash | 1,090ms | 2,698ms | 2.5x | MoE overhead on Apple Silicon unified memory |
| Qwen3.5-27B | 6,411ms | 41,634ms | 6.5x | Think-block runaway on some scenarios (command_execution: 269s) |

**Qwen models use native tool-call format**, not fence format. The Jinja template engine renders Qwen's `<tool_call><function=name><parameter=key>value</parameter></function></tool_call>` syntax. The parser handles this via `extract_tool_calls_from_text()` → `parse_xml()` → `parse_function_eq_format()` → `extract_named_parameters()`. This adds near-zero overhead for Qwen3.5-0.8B (1.05x) since no format conversion is needed.

**Ministral models use native function-call format** (`tool_name{json_args}`), caught by the `extract_curly_brace_tool_calls()` fallback. Pipeline overhead (~500ms) includes Jinja template rendering, tool schema injection, and response post-processing.

**For large models (27B), think-block generation dominates.** Qwen3.5-27B generates extensive `<think>` blocks before tool calls, inflating latency far beyond raw inference speed.

### Optimization Opportunities

- **Prompt caching**: The ~200ms pipeline overhead includes re-building tool schemas per request. Caching the tool prompt prefix across requests for the same model would eliminate ~50-80ms.
- **Speculative parsing**: Start fence-format parsing while tokens are still streaming instead of waiting for completion.
- **Grammar-constrained decoding**: GBNF grammars (currently opt-in) eliminate invalid tool calls at generation time, removing the need for post-hoc validation retries. Blocked by Ministral-3B Q4_K_M crash.

## Running Benchmarks

```bash
# Single model
arkavo tool-bench --model Ministral-3-3B

# All cached models
arkavo tool-bench --all

# JSON report output
arkavo tool-bench --all --output report.json
```
