# Tool-Calling Benchmarks Across Local Models

Benchmarked using `arkavo tool-bench` with 8 standardized scenarios: single-param, multi-param, no-param, enum, file path, command execution, should-not-call, and multi-type params. Five test tools registered (get_weather, read_file, search, get_time, run_command).

## Results (2026-03-08)

| Model | Size | Parse | Tool Name | Params | Avg Latency |
|-------|------|-------|-----------|--------|-------------|
| Qwen3-0.6B | 0.6B | 8/8 | 7/8 | 8/8 | 171ms |
| Qwen3.5-0.8B | 0.8B | 1/8 | 1/8 | 1/8 | 22444ms |
| **Ministral-3-3B** | **3B** | **8/8** | **8/8** | **8/8** | **422ms** |
| GLM-4.7-Flash | 4.7B | 8/8 | 8/8 | 8/8 | 782ms |
| Ministral-3-8B | 8B | 8/8 | 8/8 | 8/8 | 783ms |
| Qwen3.5-27B | 27B | 8/8 | 8/8 | 8/8 | 5868ms |

## Key Findings

**Qwen3.5-0.8B is critically broken for tool calling.** 1/8 accuracy at 22.4s latency — both the slowest and least accurate model. Qwen3-0.6B (7/8 at 171ms) is vastly superior for ultra-lightweight deployments. Qwen3.5-0.8B was removed as the default chat model in favor of Ministral-3B.

**All 3B+ models achieve perfect 8/8.** Ministral-3-3B, GLM-4.7-Flash, Ministral-3-8B, and Qwen3.5-27B all produce valid fence-format tool calls with correct tool names and parameters on every scenario.

**Ministral-3-3B is the default chat model.** Perfect 8/8 scores at 422ms average — the best latency/accuracy tradeoff. Selected by `fastest_local_model()` for all chat path tool calling.

**GLM-4.7-Flash and Ministral-8B are nearly identical.** 782ms vs 783ms, both 8/8 perfect. GLM-4.7 requires 32GB RAM (MoE architecture).

**27B models are slow on Apple Silicon.** Qwen3.5-27B takes 5.9s average per tool call. Suitable for batch/offline tasks, not real-time agentic loops.

## Improvements Implemented

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

## Running Benchmarks

```bash
# Single model
arkavo tool-bench --model Ministral-3-3B

# All cached models
arkavo tool-bench --all

# JSON report output
arkavo tool-bench --all --output report.json
```
