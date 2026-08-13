# Grok 4.5 / 4.6 Support Plan

## Grok 4.6 update (2026-08-13)

The flagship xAI arm is now **Grok 4.6**. `ModelChoice::Grok46` is the
low-effort Thompson arm (same role as the former `Grok45`).
`ModelChoice::Grok46Xhigh` is a companion arm that forces
`reasoning.effort = "xhigh"` (Grok 4.6+ only; 4.5 treated `xhigh` as `high`).
Persisted `"Grok45"` traces deserialize as `Grok46`. Name aliases `grok-4.5`,
`grok-4.5-latest`, and `grok45` resolve to `Grok46`. `XAI_REASONING_EFFORT`
accepts `low` / `medium` / `high` / `xhigh`.

| Item | Value |
|------|--------|
| Model ID | `grok-4.6` (aliases: `grok-4.6-latest`, `grok-build-latest`, `grok`) |
| xhigh arm | `grok-4.6-xhigh` (API model still `grok-4.6`) |
| Pricing | **$2.00 / $0.50 cached / $6.00** per 1M tokens below 200k prompt tokens |
| Context | 500k tokens |
| Arkavo reasoning default | `low` on `Grok46`; `xhigh` on `Grok46Xhigh` |

## Goal

Add **xAI Grok 4.5** (now superseded by 4.6) as a routable cloud model arm via the **xAI Responses API**
(`POST /v1/responses`) using `ResponsesProvider` in `arkavo-llm`. Chat Completions
remains available through `OpenAIProvider` for generic OpenAI-compatible hosts,
but Grok routing intentionally uses Responses for reasoning effort, function-call
items, and optional SSE streaming.

## Facts (xAI API, as of research)

| Item | Value |
|------|--------|
| Model ID | `grok-4.5` (aliases: `grok-4.5-latest`, `grok-build-latest`) |
| Base URL | `https://api.x.ai/v1` |
| Endpoint | `POST /v1/responses` (Responses API) |
| Auth | `Authorization: Bearer $XAI_API_KEY` |
| Context | 500k tokens |
| Pricing | **$2.00 / $0.50 cached / $6.00** per 1M tokens (input / cached input / output) |
| Wire format | xAI Responses (not Chat Completions for the Grok arm) |
| Capabilities | tools, structured outputs, configurable reasoning, text+image input |
| Arkavo reasoning default | `low` (xAI platform default is `high`; low reduces agent latency) |

Env vars:
- `XAI_API_KEY` (required)
- `XAI_BASE_URL` (optional, default `https://api.x.ai/v1`)
- `XAI_STORE` (optional; `1`/`true` enables server-side store for response-id chaining)
- `XAI_PROMPT_CACHE_KEY` (optional; stable key for multi-turn cache hits)
- `XAI_REASONING_EFFORT` (optional; `low` / `medium` / `high` / `xhigh`, default `low`)

## Approach

Prefer Responses over Chat Completions for Grok 4.5. Implement under
`arkavo-llm` (`providers/xai_responses/`), not a separate `arkavo-xai` crate.
Gate with feature `xai` that enables `arkavo-llm/llm-remote`.

### Multi-turn (v1)

The standard `Provider` path re-sends the full transcript each turn
(`previous_response_id` is not used by the agent loop). Server-side chaining via
`continue_with_tool_outputs` is available when `store` is on. Streaming updates
`last_response_id` from `response.completed` events.

## Touchpoints (checklist)

### 1. Model registry — `arkavo-router/src/decision.rs`
- [x] Add `ModelChoice::Grok45`
- [x] `name()` → `"grok-4.5"`
- [x] `from_name()` aliases: `grok-4.5`, `grok-4.5-latest`, `grok-build-latest`, `grok45`, `grok`
- [x] `family()` → `"grok"` / `provider()` → `"xai"`
- [x] `is_grok()`, include in `ALL_CLOUD`, `is_cloud()`, `capability()` Large
- [x] `display_name()` → `"Grok 4.5"`
- [x] Cost estimate: $2.00 in / $6.00 out per MTok
- [x] Latency estimate: ~6–8s (reasoning-capable)
- [x] Fallback chain: ClaudeSonnet / GeminiFlash / LocalMinistral8B
- [x] Unit tests for name, aliases, cost, cloud membership

### 2. Availability + instantiation
- [x] `provider.rs`: `is_xai_available()` / `XAI_API_KEY` + `cfg(feature = "xai")`
- [x] `instantiate_provider`: build `ResponsesProvider` with base `https://api.x.ai/v1`
- [x] `selector.rs` `ProviderAvailability.xai` + `feasible_models()` push `Grok45`
- [x] `has_cloud()` includes xai
- [x] Update availability test helpers

### 3. Feature flags
- [x] `arkavo-router`: `xai = ["arkavo-llm/llm-remote"]` (default + windows)
- [x] `arkavo-cli`: `xai = ["arkavo-llm/llm-remote", "arkavo-router/xai", "arkavo-ui-core/xai"]`
- [x] `arkavo` binary: `xai = ["arkavo-cli/xai"]`, add to `default`
- [x] `arkavo-ui-core`: matching `xai` feature

### 4. Client construction call sites
- [x] `arkavo-cli/src/commands/ui.rs` — `create_client_from_routing`
- [x] `arkavo-ui-core/src/llm_integration.rs`
- [x] Exhaustive `match` on `ModelChoice` (architect executor, tool extraction, planes, quality notes)

### 5. Cost / budget / ranking
- [x] Static cost arm in `estimate_cost`
- [x] Architect `estimate_actual_cost` prices Grok ($2/$6)
- [x] `selector_quality.rs` quality note string for Grok45
- [x] Default `PricingEntry` sample for Grok in budget tests

### 6. Security / egress
- [x] Allow `https://api.x.ai` in `secure_http` egress filter

### 7. Tests
- [x] Unit: decision/selector/provider availability (no network)
- [x] Unit: Responses convert/SSE helpers against production code
- [x] Live e2e: `e2e_grok.rs` (router/`LlmClient` path) and `e2e_xai_responses.rs` (create id + stream)
- [x] Live tests `#[ignore]`, skip if no `XAI_API_KEY`, soft-skip on 429/quota

### 8. Specs (optional but preferred)
- [x] Router/budget scenarios covered by parent behavior-spec expansion

## Out of scope (v1)
- Image generation (`grok-imagine-*`)
- Voice / realtime
- Multi-agent Grok variants (`grok-4.20-multi-agent-*`)
- Reasoning-tier split (single arm first, Thompson Sampling learns fit)
- Dedicated `arkavo-xai` crate
- Agent-loop `previous_response_id` chaining (available as API; not wired through `Provider`)

## Implementation order
1. Branch from `feature/spec-coverage-budget-router` ✅
2. `ModelChoice` + unit tests
3. Feature flags + availability + Responses provider instantiation
4. CLI / UI client wiring
5. Exhaustive match fixups + quality notes
6. Egress allowlist
7. Live e2e test
8. Build + targeted nextest
9. Commit

## Risk notes
- **Exhaustive matches**: adding an enum variant forces many match arms; compile will list them.
- **Cold-start exploration**: one Thompson arm only (same as GLM).
- **Pricing above 200k context**: docs note higher rates past 200k; v1 uses flat list rates for estimates; real spend from `usage` when available.
- **Secrets**: never commit `XAI_API_KEY`.
- **Store default**: `store=false` for privacy; opt in with `XAI_STORE=1` for response-id chaining.
