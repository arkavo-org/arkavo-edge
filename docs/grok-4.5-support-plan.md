# Grok 4.5 Support Plan

## Goal

Add **xAI Grok 4.5** as a routable cloud model arm, following the same pattern as **GLM-5.2**: OpenAI-compatible chat completions via the existing `OpenAIProvider`, no new provider crate.

## Facts (xAI API, as of research)

| Item | Value |
|------|--------|
| Model ID | `grok-4.5` (aliases: `grok-4.5-latest`, `grok-build-latest`) |
| Base URL | `https://api.x.ai/v1` |
| Auth | `Authorization: Bearer $XAI_API_KEY` |
| Context | 500k tokens |
| Pricing | **$2.00 / $0.50 cached / $6.00** per 1M tokens (input / cached input / output) |
| Wire format | OpenAI chat completions + function calling |
| Capabilities | tools, structured outputs, configurable reasoning, text+image input |

Env vars:
- `XAI_API_KEY` (required)
- `XAI_BASE_URL` (optional, default `https://api.x.ai/v1`)

## Approach

Mirror GLM-5.2 (#632). Reuse `OpenAIProvider`. Gate with feature `xai` (or `grok`) that enables `arkavo-llm/llm-remote`.

## Touchpoints (checklist)

### 1. Model registry — `arkavo-router/src/decision.rs`
- [ ] Add `ModelChoice::Grok45`
- [ ] `name()` → `"grok-4.5"`
- [ ] `from_name()` aliases: `grok-4.5`, `grok-4.5-latest`, `grok-build-latest`, `grok45`, `grok`
- [ ] `family()` → `"grok"` / `provider()` → `"xai"`
- [ ] `is_grok()`, include in `ALL_CLOUD`, `is_cloud()`, `capability()` Large
- [ ] `display_name()` → `"Grok 4.5"`
- [ ] Cost estimate: $2.00 in / $6.00 out per MTok
- [ ] Latency estimate: ~6–8s (reasoning-capable)
- [ ] Fallback chain: ClaudeSonnet / GeminiFlash / LocalMinistral8B
- [ ] Unit tests for name, aliases, cost, cloud membership

### 2. Availability + instantiation
- [ ] `provider.rs`: `is_xai_available()` / `XAI_API_KEY` + `cfg(feature = "xai")`
- [ ] `instantiate_provider`: build `OpenAIProvider` with base `https://api.x.ai/v1`
- [ ] `selector.rs` `ProviderAvailability.xai` + `feasible_models()` push `Grok45`
- [ ] `has_cloud()` includes xai
- [ ] Update availability test helpers

### 3. Feature flags
- [ ] `arkavo-router`: `xai = ["arkavo-llm/llm-remote"]` (default + windows)
- [ ] `arkavo-cli`: `xai = ["arkavo-llm/llm-remote", "arkavo-router/xai", "arkavo-ui-core/xai"]`
- [ ] `arkavo` binary: `xai = ["arkavo-cli/xai"]`, add to `default`
- [ ] `arkavo-ui-core`: matching `xai` feature

### 4. Client construction call sites
- [ ] `arkavo-cli/src/commands/ui.rs` — `create_client_from_routing`
- [ ] `arkavo-ui-core/src/llm_integration.rs`
- [ ] Any other exhaustive `match` on `ModelChoice` (architect executor, tool extraction, planes, quality notes)

### 5. Cost / budget / ranking
- [ ] Static cost arm in `estimate_cost` (above)
- [ ] `selector_quality.rs` quality note string for Grok45
- [ ] Orchestrator / learning arms if they hard-code cloud lists
- [ ] Optional: default `PricingEntry` for manifest pricing tests

### 6. Security / egress
- [ ] Allow `https://api.x.ai` in `secure_http` egress filter (and protocol egress if separate)

### 7. Tests
- [ ] Unit: decision/selector/provider availability (no network)
- [ ] Live e2e: `crates/arkavo-llm/tests/e2e_grok.rs` (`#[ignore]`, skip if no `XAI_API_KEY`, skip on 429/quota)
- [ ] Cost reconciliation smoke with usage tokens when live

### 8. Specs (optional but preferred)
- [ ] Router/budget scenarios for Grok availability + pricing authority
- [ ] `#[spec(...)]` on new tests if scenarios added

## Out of scope (v1)
- Image generation (`grok-imagine-*`)
- Voice / realtime
- Multi-agent Grok variants (`grok-4.20-multi-agent-*`)
- Reasoning-tier split (follow GLM pattern: single arm first, Thompson Sampling learns fit)
- Dedicated `arkavo-xai` crate

## Implementation order
1. Branch from `feature/spec-coverage-budget-router` ✅
2. `ModelChoice` + unit tests
3. Feature flags + availability + provider instantiation
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
