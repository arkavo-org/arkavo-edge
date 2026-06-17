use crate::classifier::TaskCategory;
use crate::learning::DecisionTrace;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Model capability tier based on parameter count
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlannerTier {
    /// Less than 2B parameters - info gathering, simple tasks
    Small,
    /// 2-7B parameters - planning, tool use, reasoning
    Medium,
    /// Greater than 7B parameters - complex planning, multi-step
    Large,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelChoice {
    GeminiFlash,
    /// Gemini 3.5 Flash with the **low** thinking tier (budget 512 tokens).
    /// Production daily-driver — matches AA-recommended pinned thinking that
    /// avoids the dynamic-thinking timeout class.
    Gemini35Flash,
    /// Gemini 3.5 Flash with thinking **disabled** (budget 0). Fast and
    /// cheap; in practice the model becomes too conservative for tool-call
    /// loops, so prefer `Gemini35Flash` for routine routing.
    Gemini35FlashMinimal,
    /// Gemini 3.5 Flash with the **medium** thinking tier (budget 2048).
    /// Matches the server default; spend climbs and bimodal latency
    /// reappears, but quality on complex planning is meaningfully higher.
    Gemini35FlashMedium,
    /// Gemini 3.5 Flash with the **high** thinking tier (budget 8192).
    /// Use for explicit deliberation paths only — Thompson Sampling
    /// should learn which task types justify the cost/latency hit.
    Gemini35FlashHigh,
    GeminiPro,
    ClaudeSonnet,
    ClaudeOpus,
    /// Claude Fable 5 - Anthropic's most capable tier, above Opus. 1M context,
    /// 128K max output, adaptive-thinking-only API surface. At $10/$50 per
    /// MTok (2x Opus) it is the escalation ceiling and explicit-selection
    /// premium arm, never a silent default.
    ClaudeFable5,
    /// Qwen3.5-0.8B - fast DeltaNet, TØRG-compatible (preferred default)
    LocalQwen3,
    /// Ministral-3B - TØRG-compatible, higher quality
    LocalMinistral3B,
    /// Ministral-8B - TØRG-compatible, high quality
    LocalMinistral8B,
    /// Qwen3.5-9B - 9B dense model, good balance of quality and speed
    LocalQwen35_9B,
    /// Qwen3.5-27B - 27B dense model, requires 48GB+ RAM
    LocalQwen35_27B,
    /// Qwen3.6-35B-A3B - 35B MoE with 3B active, native vision, 262K context
    LocalQwen36A3B,
    /// GLM-4.7-Flash - 30B MoE reasoning model, requires 32GB+ RAM
    LocalGlm47Flash,
    /// Gemma-4-E2B - 2.3B active (5.1B total with PLE), multimodal edge model
    LocalGemma4E2B,
    /// Gemma-4-E4B - 4.5B active (8B total with PLE), multimodal edge model
    LocalGemma4E4B,
    /// Gemma-4-26B-A4B - MoE 4B active/26B total, 128 experts
    LocalGemma4_26B,
    /// Gemma-4-31B - Dense 31B, stronger per-token reasoning than MoE
    LocalGemma4_31B,
    /// Gemma-4-12B - Dense 12B, multimodal, strong reasoning for its size
    LocalGemma4_12B,
    /// Legacy: Gemma-3-270M (if cached)
    LocalGemma270M,
    /// Legacy: Gemma-3-4B (if cached)
    LocalGemma4B,
    /// Legacy: Gemma-3-12B (if cached)
    LocalGemma12B,
    LocalDeepSeekCoder,
    /// DeepSeek V3.2 - daily driver with tool support
    DeepSeekV32,
    /// DeepSeek V3.2-Speciale - planning/reasoning only (no tools)
    DeepSeekV32Speciale,
    /// Kimi K2.5 - 256K context, thinking mode support
    KimiK2,
    /// GLM-5.2 (Zhipu AI / Z.ai) - OpenAI-compatible cloud reasoning model.
    /// Routed through the generic OpenAI-compatible adapter (base URL +
    /// model string + `GLM_API_KEY`). Introduced as a single Thompson
    /// Sampling arm to respect the cold-start exploration cap; a thinking-tier
    /// (High/Max) split can follow once this base arm graduates probation.
    Glm52,
}

impl ModelChoice {
    pub fn name(&self) -> &str {
        match self {
            Self::GeminiFlash => "gemini-flash-latest",
            // Thompson Sampling sees each thinking tier as a distinct arm,
            // so the canonical name carries the tier suffix. `gemini_api_model()`
            // strips it back to the API model id for REST calls.
            Self::Gemini35Flash => "gemini-3.5-flash",
            Self::Gemini35FlashMinimal => "gemini-3.5-flash-minimal",
            Self::Gemini35FlashMedium => "gemini-3.5-flash-medium",
            Self::Gemini35FlashHigh => "gemini-3.5-flash-high",
            Self::GeminiPro => "gemini-3-pro-preview",
            Self::ClaudeSonnet => "claude-sonnet-4-5-20250929",
            // Dateless ids are pinned snapshots, not evergreen pointers.
            // Opus 4.8 is adaptive-thinking-only (same surface as Fable 5);
            // the provider gates sampling params off via `uses_adaptive_thinking`.
            Self::ClaudeOpus => "claude-opus-4-8",
            // Alias, not a dated snapshot — Anthropic serves Fable 5 under
            // the bare id only.
            Self::ClaudeFable5 => "claude-fable-5",
            Self::LocalQwen3 => "qwen3.5-0.8b",
            Self::LocalMinistral3B => "ministral-3b",
            Self::LocalMinistral8B => "ministral-8b",
            Self::LocalQwen35_9B => "qwen3.5-9b",
            Self::LocalQwen35_27B => "qwen3.5-27b",
            Self::LocalQwen36A3B => "qwen3.6-35b-a3b",
            Self::LocalGlm47Flash => "glm-4.7-flash",
            Self::LocalGemma4E2B => "gemma-4-e2b",
            Self::LocalGemma4E4B => "gemma-4-e4b",
            Self::LocalGemma4_26B => "gemma-4-26b-a4b",
            Self::LocalGemma4_31B => "gemma-4-31b",
            Self::LocalGemma4_12B => "gemma-4-12b",
            Self::LocalGemma270M => "gemma-3-270m-it",
            Self::LocalGemma4B => "gemma-3-4b-it",
            Self::LocalGemma12B => "gemma-3-12b-it",
            Self::LocalDeepSeekCoder => "deepseek-coder-v2-lite-instruct",
            Self::DeepSeekV32 | Self::DeepSeekV32Speciale => "deepseek-chat",
            Self::KimiK2 => "kimi-k2.5",
            Self::Glm52 => "glm-5.2",
        }
    }

    /// Model family identifier for prompt advisor lookups
    pub fn family(&self) -> &str {
        match self {
            Self::LocalQwen3
            | Self::LocalQwen35_9B
            | Self::LocalQwen35_27B
            | Self::LocalQwen36A3B => "qwen",
            Self::LocalMinistral3B | Self::LocalMinistral8B => "mistral",
            Self::LocalGlm47Flash | Self::Glm52 => "glm",
            Self::LocalGemma4E2B
            | Self::LocalGemma4E4B
            | Self::LocalGemma4_26B
            | Self::LocalGemma4_31B
            | Self::LocalGemma4_12B
            | Self::LocalGemma270M
            | Self::LocalGemma4B
            | Self::LocalGemma12B => "gemma",
            Self::LocalDeepSeekCoder | Self::DeepSeekV32 | Self::DeepSeekV32Speciale => "deepseek",
            Self::GeminiFlash
            | Self::Gemini35Flash
            | Self::Gemini35FlashMinimal
            | Self::Gemini35FlashMedium
            | Self::Gemini35FlashHigh
            | Self::GeminiPro => "google",
            Self::ClaudeSonnet | Self::ClaudeOpus | Self::ClaudeFable5 => "anthropic",
            Self::KimiK2 => "kimi",
        }
    }

    /// Resolve a model name string to a ModelChoice (reverse of `name()`).
    /// Used as a hint from AGENTS.md `model:` field.
    pub fn from_name(name: &str) -> Option<Self> {
        let name = &name.to_lowercase();
        match name.as_str() {
            "qwen3.5-0.8b" | "qwen3-0.6b" => Some(Self::LocalQwen3),
            "ministral-3b" => Some(Self::LocalMinistral3B),
            "ministral-8b" => Some(Self::LocalMinistral8B),
            "qwen3.5-9b" => Some(Self::LocalQwen35_9B),
            "qwen3.5-27b" => Some(Self::LocalQwen35_27B),
            "qwen3.6-35b-a3b" | "qwen3.6" | "qwen36" => Some(Self::LocalQwen36A3B),
            "glm-4.7-flash" => Some(Self::LocalGlm47Flash),
            "glm-5.2" | "glm5.2" | "glm-5" | "glm52" => Some(Self::Glm52),
            "gemma-4-e2b" => Some(Self::LocalGemma4E2B),
            "gemma-4-e4b" => Some(Self::LocalGemma4E4B),
            "gemma-4-26b-a4b" => Some(Self::LocalGemma4_26B),
            "gemma-4-31b" => Some(Self::LocalGemma4_31B),
            "gemma-4-12b" => Some(Self::LocalGemma4_12B),
            "gemma-3-270m-it" => Some(Self::LocalGemma270M),
            "gemma-3-4b-it" => Some(Self::LocalGemma4B),
            "gemma-3-12b-it" => Some(Self::LocalGemma12B),
            "deepseek-coder-v2-lite-instruct" => Some(Self::LocalDeepSeekCoder),
            "deepseek-chat" => Some(Self::DeepSeekV32),
            "kimi-k2.5" => Some(Self::KimiK2),
            "gemini-flash-latest" => Some(Self::GeminiFlash),
            "gemini-3.5-flash"
            | "gemini-3.5-flash-latest"
            | "gemini-3.5"
            | "gemini35flash"
            // gemini-3.1 Live/TTS helper variants share the 3.5 stack's
            // routing / billing surface — alias them to the Flash entry so
            // they pass availability + cost checks.
            | "gemini-3.1-flash-live"
            | "gemini-3.1-flash-tts"
            // Explicit "low" tier alias resolves to the default Gemini35Flash arm.
            | "gemini-3.5-flash-low" => Some(Self::Gemini35Flash),
            "gemini-3.5-flash-minimal" | "gemini-3.5-flash-off" => {
                Some(Self::Gemini35FlashMinimal)
            }
            "gemini-3.5-flash-medium" => Some(Self::Gemini35FlashMedium),
            "gemini-3.5-flash-high" => Some(Self::Gemini35FlashHigh),
            "gemini-3-pro-preview" => Some(Self::GeminiPro),
            _ if name.contains("claude-sonnet") => Some(Self::ClaudeSonnet),
            _ if name.contains("claude-opus") => Some(Self::ClaudeOpus),
            _ if name.contains("fable") => Some(Self::ClaudeFable5),
            _ => None,
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(
            self,
            Self::LocalQwen3
                | Self::LocalMinistral3B
                | Self::LocalMinistral8B
                | Self::LocalQwen35_9B
                | Self::LocalQwen35_27B
                | Self::LocalQwen36A3B
                | Self::LocalGlm47Flash
                | Self::LocalGemma4E2B
                | Self::LocalGemma4E4B
                | Self::LocalGemma4_26B
                | Self::LocalGemma4_31B
                | Self::LocalGemma4_12B
                | Self::LocalGemma270M
                | Self::LocalGemma4B
                | Self::LocalGemma12B
                | Self::LocalDeepSeekCoder
        )
    }

    pub fn is_cloud(&self) -> bool {
        !self.is_local()
    }

    /// All local model variants, ordered smallest-first for fast-path preference.
    pub const ALL_LOCAL: &[Self] = &[
        Self::LocalGemma270M,
        Self::LocalQwen3,
        Self::LocalGemma4E2B,
        Self::LocalMinistral3B,
        Self::LocalGemma4B,
        Self::LocalGemma4E4B,
        Self::LocalMinistral8B,
        Self::LocalQwen35_9B,
        Self::LocalGemma12B,
        Self::LocalGemma4_12B,
        Self::LocalDeepSeekCoder,
        Self::LocalGemma4_26B,
        Self::LocalGlm47Flash,
        Self::LocalGemma4_31B,
        Self::LocalQwen36A3B,
        Self::LocalQwen35_27B,
    ];

    pub fn is_anthropic(&self) -> bool {
        matches!(
            self,
            Self::ClaudeSonnet | Self::ClaudeOpus | Self::ClaudeFable5
        )
    }

    pub fn is_gemini(&self) -> bool {
        matches!(
            self,
            Self::GeminiFlash
                | Self::Gemini35Flash
                | Self::Gemini35FlashMinimal
                | Self::Gemini35FlashMedium
                | Self::Gemini35FlashHigh
                | Self::GeminiPro
        )
    }

    /// Underlying Gemini API model id (strips the per-arm thinking-tier
    /// suffix). Returns `None` for non-Gemini variants.
    pub fn gemini_api_model(&self) -> Option<&'static str> {
        match self {
            Self::GeminiFlash => Some("gemini-flash-latest"),
            Self::Gemini35Flash
            | Self::Gemini35FlashMinimal
            | Self::Gemini35FlashMedium
            | Self::Gemini35FlashHigh => Some("gemini-3.5-flash"),
            Self::GeminiPro => Some("gemini-3-pro-preview"),
            _ => None,
        }
    }

    /// `thinkingBudget` to pass to the Gemini API for this arm. `None`
    /// means "leave the server at its default" (which on Gemini 3.5 Flash
    /// is dynamic thinking — the latency-spike regime we want to avoid).
    /// Per the Artificial Analysis pre-release: minimal / low / medium /
    /// high tiers are exposed; we map them to concrete token caps so
    /// Thompson Sampling can learn the cost/quality tradeoff per task type.
    pub fn gemini_thinking_budget(&self) -> Option<i32> {
        match self {
            Self::Gemini35FlashMinimal => Some(0),
            Self::Gemini35Flash => Some(512),
            Self::Gemini35FlashMedium => Some(2048),
            Self::Gemini35FlashHigh => Some(8192),
            _ => None,
        }
    }

    pub fn is_deepseek(&self) -> bool {
        matches!(
            self,
            Self::DeepSeekV32 | Self::DeepSeekV32Speciale | Self::LocalDeepSeekCoder
        )
    }

    pub fn is_kimi(&self) -> bool {
        matches!(self, Self::KimiK2)
    }

    /// Cloud GLM (Zhipu AI / Z.ai) arm, reached via the OpenAI-compatible
    /// adapter. Distinct from the local `LocalGlm47Flash` GGUF model.
    pub fn is_glm(&self) -> bool {
        matches!(self, Self::Glm52)
    }

    pub fn provider(&self) -> &str {
        match self {
            Self::GeminiFlash
            | Self::Gemini35Flash
            | Self::Gemini35FlashMinimal
            | Self::Gemini35FlashMedium
            | Self::Gemini35FlashHigh
            | Self::GeminiPro => "google",
            Self::ClaudeSonnet | Self::ClaudeOpus | Self::ClaudeFable5 => "anthropic",
            Self::LocalQwen3
            | Self::LocalQwen35_9B
            | Self::LocalQwen35_27B
            | Self::LocalQwen36A3B => "local-qwen",
            Self::LocalMinistral3B | Self::LocalMinistral8B => "local-ministral",
            Self::LocalGlm47Flash => "local-glm",
            Self::LocalGemma4E2B
            | Self::LocalGemma4E4B
            | Self::LocalGemma4_26B
            | Self::LocalGemma4_31B
            | Self::LocalGemma4_12B
            | Self::LocalGemma270M
            | Self::LocalGemma4B
            | Self::LocalGemma12B => "local-gemma",
            Self::LocalDeepSeekCoder => "local-deepseek",
            Self::DeepSeekV32 | Self::DeepSeekV32Speciale => "deepseek",
            Self::KimiK2 => "kimi",
            Self::Glm52 => "zhipu",
        }
    }

    /// Get the capability tier for this model based on parameter count
    pub fn capability(&self) -> PlannerTier {
        match self {
            // Small: < 2B parameters
            Self::LocalQwen3 | Self::LocalGemma270M => PlannerTier::Small,
            // Medium: 2-7B parameters
            Self::LocalGemma4E2B
            | Self::LocalMinistral3B
            | Self::LocalGemma4B
            | Self::LocalGemma4E4B => PlannerTier::Medium,
            // Large: > 7B parameters or cloud models
            Self::LocalGemma4_26B
            | Self::LocalGemma4_31B
            | Self::LocalGemma4_12B
            | Self::LocalMinistral8B
            | Self::LocalQwen35_9B
            | Self::LocalQwen35_27B
            | Self::LocalQwen36A3B
            | Self::LocalGlm47Flash
            | Self::LocalGemma12B
            | Self::LocalDeepSeekCoder
            | Self::GeminiFlash
            | Self::Gemini35Flash
            | Self::Gemini35FlashMinimal
            | Self::Gemini35FlashMedium
            | Self::Gemini35FlashHigh
            | Self::GeminiPro
            | Self::ClaudeSonnet
            | Self::ClaudeOpus
            | Self::ClaudeFable5
            | Self::DeepSeekV32
            | Self::DeepSeekV32Speciale
            | Self::KimiK2
            | Self::Glm52 => PlannerTier::Large,
        }
    }

    /// HuggingFace repo ID for local models (None for cloud)
    pub fn repo_id(&self) -> Option<&'static str> {
        match self {
            Self::LocalQwen3 => Some("unsloth/Qwen3.5-0.8B-GGUF"),
            Self::LocalMinistral3B => Some("mistralai/Ministral-3-3B-Instruct-2512-GGUF"),
            Self::LocalMinistral8B => Some("mistralai/Ministral-3-8B-Instruct-2512-GGUF"),
            Self::LocalQwen35_9B => Some("unsloth/Qwen3.5-9B-GGUF"),
            Self::LocalQwen35_27B => Some("unsloth/Qwen3.5-27B-GGUF"),
            Self::LocalQwen36A3B => Some("unsloth/Qwen3.6-35B-A3B-GGUF"),
            Self::LocalGlm47Flash => Some("unsloth/GLM-4.7-Flash-GGUF"),
            Self::LocalGemma4E2B => Some("unsloth/gemma-4-E2B-it-GGUF"),
            Self::LocalGemma4E4B => Some("ggml-org/gemma-4-E4B-it-GGUF"),
            Self::LocalGemma4_26B => Some("ggml-org/gemma-4-26B-A4B-it-GGUF"),
            Self::LocalGemma4_31B => Some("ggml-org/gemma-4-31B-it-GGUF"),
            Self::LocalGemma4_12B => Some("ggml-org/gemma-4-12B-it-GGUF"),
            Self::LocalGemma270M => Some("unsloth/gemma-3-270m-it-GGUF"),
            Self::LocalGemma4B => Some("unsloth/gemma-3-4b-it-GGUF"),
            Self::LocalGemma12B => Some("unsloth/gemma-3-12b-it-GGUF"),
            Self::LocalDeepSeekCoder => Some("bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF"),
            _ => None,
        }
    }

    /// GGUF filename in the HuggingFace repo (None for cloud)
    pub fn gguf_filename(&self) -> Option<&'static str> {
        match self {
            Self::LocalQwen3 => Some("Qwen3.5-0.8B-Q4_K_M.gguf"),
            Self::LocalMinistral3B => Some("Ministral-3-3B-Instruct-2512-Q5_K_M.gguf"),
            Self::LocalMinistral8B => Some("Ministral-3-8B-Instruct-2512-Q5_K_M.gguf"),
            Self::LocalQwen35_9B => Some("Qwen3.5-9B-Q4_K_M.gguf"),
            Self::LocalQwen35_27B => Some("Qwen3.5-27B-UD-Q6_K_XL.gguf"),
            Self::LocalQwen36A3B => Some("Qwen3.6-35B-A3B-UD-Q4_K_M.gguf"),
            Self::LocalGlm47Flash => Some("GLM-4.7-Flash-Q4_K_M.gguf"),
            Self::LocalGemma4E2B => Some("gemma-4-E2B-it-Q4_K_M.gguf"),
            Self::LocalGemma4E4B => Some("gemma-4-e4b-it-Q4_K_M.gguf"),
            Self::LocalGemma4_26B => Some("gemma-4-26B-A4B-it-Q4_K_M.gguf"),
            Self::LocalGemma4_31B => Some("gemma-4-31B-it-Q4_K_M.gguf"),
            Self::LocalGemma4_12B => Some("gemma-4-12B-it-Q4_K_M.gguf"),
            Self::LocalGemma270M => Some("gemma-3-270m-it-Q4_0.gguf"),
            Self::LocalGemma4B => Some("gemma-3-4b-it-Q4_0.gguf"),
            Self::LocalGemma12B => Some("gemma-3-12b-it-Q4_0.gguf"),
            Self::LocalDeepSeekCoder => Some("DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf"),
            _ => None,
        }
    }

    /// Cache directory name derived from repo_id ("org/repo" → "models--org--repo")
    pub fn cache_dir_name(&self) -> Option<String> {
        self.repo_id()
            .map(|r| format!("models--{}", r.replace('/', "--")))
    }

    /// Download instruction for error messages
    pub fn download_hint(&self) -> Option<String> {
        match (self.repo_id(), self.gguf_filename()) {
            (Some(repo), Some(file)) => Some(format!("huggingface-cli download {repo} {file}")),
            _ => None,
        }
    }

    /// Approximate model size in bytes
    pub fn size_bytes(&self) -> u64 {
        match self {
            Self::LocalQwen3 => 550_000_000,
            Self::LocalMinistral3B => 6_000_000_000,
            Self::LocalMinistral8B => 5_500_000_000,
            Self::LocalQwen35_9B => 6_000_000_000,
            Self::LocalQwen35_27B => 23_000_000_000,
            Self::LocalQwen36A3B => 22_100_000_000,
            Self::LocalGlm47Flash => 20_000_000_000,
            Self::LocalGemma4E2B => 3_000_000_000,
            Self::LocalGemma4E4B => 5_000_000_000,
            Self::LocalGemma4_26B => 17_000_000_000,
            Self::LocalGemma4_31B => 20_000_000_000,
            Self::LocalGemma4_12B => 7_400_000_000,
            Self::LocalGemma270M => 200_000_000,
            Self::LocalGemma4B => 2_500_000_000,
            Self::LocalGemma12B => 7_000_000_000,
            Self::LocalDeepSeekCoder => 9_000_000_000,
            _ => 0,
        }
    }

    /// Whether this model supports vision (multimodal) via mmproj
    ///
    /// Only models with dedicated vision projector support should auto-load
    /// mmproj files. Small routing models skip vision to avoid overhead.
    /// Autoresearch-discovered optimal inference parameters.
    /// Returns (temperature, top_p, ThinkingMode) or None for untuned models.
    pub fn optimal_sampling(&self) -> Option<(f32, f32, arkavo_llm::ThinkingMode)> {
        use arkavo_llm::ThinkingMode;
        match self {
            Self::LocalQwen3 => Some((0.9, 1.0, ThinkingMode::Off)),
            Self::LocalMinistral3B => Some((0.9, 1.0, ThinkingMode::Off)),
            Self::LocalMinistral8B => Some((0.3, 0.8, ThinkingMode::Off)),
            Self::LocalQwen35_9B => Some((0.7, 0.9, ThinkingMode::On)),
            Self::LocalGlm47Flash => Some((0.9, 1.0, ThinkingMode::On)),
            Self::LocalQwen35_27B => Some((0.1, 0.8, ThinkingMode::On)),
            // Qwen 3.6 has no /think /nothink soft-switch; thinking-mode default breaks
            // grammar root. Disable thinking so model emits tool calls directly.
            Self::LocalQwen36A3B => Some((0.7, 0.9, ThinkingMode::Off)),
            // Gemma-4 MoE: thinking burns KV cache tokens at 14 tok/s,
            // making tool loops unacceptably slow. Disable thinking so
            // the model produces tool calls directly.
            Self::LocalGemma4_26B => Some((0.7, 0.9, ThinkingMode::Off)),
            Self::LocalGemma4_31B => Some((0.7, 0.9, ThinkingMode::Off)),
            Self::LocalGemma4_12B => Some((0.7, 0.9, ThinkingMode::Off)),
            Self::LocalGemma4E2B => Some((0.7, 0.9, ThinkingMode::Off)),
            Self::LocalGemma4E4B => Some((0.7, 0.9, ThinkingMode::Off)),
            _ => None,
        }
    }

    pub fn supports_vision(&self) -> bool {
        matches!(
            self,
            Self::LocalQwen35_9B
                | Self::LocalQwen35_27B
                | Self::LocalQwen36A3B
                | Self::LocalGemma4E2B
                | Self::LocalGemma4E4B
                | Self::LocalGemma4_26B
                | Self::LocalGemma4_31B
                | Self::LocalGemma4_12B
                | Self::LocalGemma4B
                | Self::LocalGemma12B
                | Self::LocalMinistral8B
        )
    }

    /// If this model exceeds `max_bytes`, return the largest local model that fits.
    /// Cloud models are not constrained (size_bytes returns 0).
    pub fn downgrade_for_budget(&self, max_bytes: u64) -> Self {
        if max_bytes == 0 || self.size_bytes() <= max_bytes {
            return self.clone();
        }

        // Local models sorted by size descending — pick the largest that fits
        let candidates = [
            Self::LocalQwen35_27B,
            Self::LocalQwen36A3B,
            Self::LocalGlm47Flash,
            Self::LocalDeepSeekCoder,
            Self::LocalGemma4_12B,
            Self::LocalGemma12B,
            Self::LocalQwen35_9B,
            Self::LocalMinistral8B,
            Self::LocalGemma4E4B,
            Self::LocalMinistral3B,
            Self::LocalGemma4B,
            Self::LocalGemma4E2B,
            Self::LocalQwen3,
            Self::LocalGemma270M,
        ];

        for candidate in &candidates {
            if candidate.size_bytes() <= max_bytes {
                return candidate.clone();
            }
        }

        // Nothing fits — fall back to smallest
        Self::LocalGemma270M
    }

    /// Human-readable display name
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::LocalQwen3 => "Qwen3.5 0.8B",
            Self::LocalMinistral3B => "Ministral 3B",
            Self::LocalMinistral8B => "Ministral 8B",
            Self::LocalQwen35_9B => "Qwen3.5 9B",
            Self::LocalQwen35_27B => "Qwen3.5 27B",
            Self::LocalQwen36A3B => "Qwen3.6 35B-A3B",
            Self::LocalGlm47Flash => "GLM-4.7-Flash",
            Self::LocalGemma4E2B => "Gemma 4 E2B",
            Self::LocalGemma4E4B => "Gemma 4 E4B",
            Self::LocalGemma4_26B => "Gemma 4 26B-A4B",
            Self::LocalGemma4_31B => "Gemma 4 31B",
            Self::LocalGemma4_12B => "Gemma 4 12B",
            Self::LocalGemma270M => "Gemma 270M",
            Self::LocalGemma4B => "Gemma 4B",
            Self::LocalGemma12B => "Gemma 12B",
            Self::LocalDeepSeekCoder => "DeepSeek Coder",
            Self::GeminiFlash => "Gemini Flash",
            Self::Gemini35Flash => "Gemini 3.5 Flash (low)",
            Self::Gemini35FlashMinimal => "Gemini 3.5 Flash (minimal)",
            Self::Gemini35FlashMedium => "Gemini 3.5 Flash (medium)",
            Self::Gemini35FlashHigh => "Gemini 3.5 Flash (high)",
            Self::GeminiPro => "Gemini Pro",
            Self::ClaudeSonnet => "Claude Sonnet",
            Self::ClaudeOpus => "Claude Opus",
            Self::ClaudeFable5 => "Claude Fable 5",
            Self::DeepSeekV32 => "DeepSeek V3.2",
            Self::DeepSeekV32Speciale => "DeepSeek V3.2 Speciale",
            Self::KimiK2 => "Kimi K2.5",
            Self::Glm52 => "GLM-5.2",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub recommended_model: ModelChoice,
    pub fallback_chain: Vec<ModelChoice>,
    pub confidence: f32,
    pub reasoning: String,
    pub estimated_cost_usd: f64,
    pub estimated_time: Duration,
    pub task_category: TaskCategory,
    pub should_compress: bool,
    pub compression_target: Option<f64>,
    /// Whether the router recommends using NGRAM spec decoding for this request.
    ///
    /// Defaults to `true` (optimistic) until the per-model rolling window fills
    /// with enough samples to detect a low accept rate. Flips to `false` when
    /// the model's rolling accept rate drops below the threshold (15% over 20
    /// requests). The provider reads this from `SamplingConfig.use_spec_decoding`.
    pub use_spec_decoding: bool,
    /// Full decision trace for learning
    pub trace: DecisionTrace,
}

impl RoutingDecision {
    pub fn new(
        model: ModelChoice,
        category: TaskCategory,
        confidence: f32,
        reasoning: String,
    ) -> Self {
        let trace = DecisionTrace::fallback(category, f64::from(confidence), model.name());
        Self::with_trace(model, category, confidence, reasoning, trace)
    }

    pub fn with_trace(
        model: ModelChoice,
        category: TaskCategory,
        confidence: f32,
        reasoning: String,
        trace: DecisionTrace,
    ) -> Self {
        let fallback_chain = Self::default_fallback_chain(&model, category);
        let estimated_cost = Self::estimate_cost(&model, category);
        let estimated_time = Self::estimate_time(&model, category);
        let (should_compress, compression_target) = Self::should_use_compression(&model, category);

        Self {
            recommended_model: model,
            fallback_chain,
            confidence,
            reasoning,
            estimated_cost_usd: estimated_cost,
            estimated_time,
            task_category: category,
            should_compress,
            compression_target,
            use_spec_decoding: true,
            trace,
        }
    }

    fn should_use_compression(model: &ModelChoice, category: TaskCategory) -> (bool, Option<f64>) {
        if model.is_local() {
            return (false, None);
        }

        match category {
            TaskCategory::FrontendUI | TaskCategory::BackendAPI | TaskCategory::TestGeneration => {
                (true, Some(0.6))
            }
            TaskCategory::Refactoring | TaskCategory::General => (true, Some(0.5)),
            _ => (false, None),
        }
    }

    fn default_fallback_chain(model: &ModelChoice, category: TaskCategory) -> Vec<ModelChoice> {
        match (model, category) {
            (ModelChoice::GeminiFlash, TaskCategory::FrontendUI) => {
                vec![
                    ModelChoice::Gemini35Flash,
                    ModelChoice::GeminiPro,
                    ModelChoice::ClaudeSonnet,
                    ModelChoice::LocalMinistral3B,
                ]
            }
            (ModelChoice::Gemini35Flash, _) => {
                vec![
                    ModelChoice::GeminiPro,
                    ModelChoice::GeminiFlash,
                    ModelChoice::ClaudeSonnet,
                    ModelChoice::LocalMinistral8B,
                ]
            }
            (ModelChoice::GeminiPro, _) => {
                vec![
                    ModelChoice::ClaudeOpus,
                    ModelChoice::Gemini35Flash,
                    ModelChoice::GeminiFlash,
                    ModelChoice::LocalMinistral8B,
                ]
            }
            (ModelChoice::ClaudeSonnet, _) => {
                vec![
                    ModelChoice::GeminiFlash,
                    ModelChoice::ClaudeOpus,
                    ModelChoice::LocalMinistral3B,
                ]
            }
            (ModelChoice::ClaudeOpus, _) => {
                vec![
                    ModelChoice::GeminiPro,
                    ModelChoice::ClaudeSonnet,
                    ModelChoice::LocalMinistral8B,
                ]
            }
            (ModelChoice::ClaudeFable5, _) => {
                vec![
                    ModelChoice::ClaudeOpus,
                    ModelChoice::GeminiPro,
                    ModelChoice::LocalMinistral8B,
                ]
            }
            // Qwen3 -> Ministral-3B -> Ministral-8B -> GLM-4.7-Flash
            (ModelChoice::LocalQwen3, _) => {
                vec![ModelChoice::LocalMinistral3B, ModelChoice::GeminiFlash]
            }
            (ModelChoice::LocalMinistral3B, _) => vec![ModelChoice::LocalMinistral8B],
            (ModelChoice::LocalMinistral8B, _) => {
                vec![ModelChoice::LocalQwen35_9B, ModelChoice::LocalQwen35_27B]
            }
            (ModelChoice::LocalQwen35_9B, _) => {
                vec![ModelChoice::LocalQwen35_27B, ModelChoice::LocalGlm47Flash]
            }
            (ModelChoice::LocalQwen35_27B, _) => {
                vec![ModelChoice::LocalGlm47Flash, ModelChoice::GeminiFlash]
            }
            (ModelChoice::LocalQwen36A3B, _) => {
                vec![ModelChoice::LocalGlm47Flash, ModelChoice::GeminiFlash]
            }
            (ModelChoice::LocalGlm47Flash, _) => vec![ModelChoice::GeminiFlash],
            // Legacy Gemma fallbacks
            (ModelChoice::LocalGemma4B, _) => vec![ModelChoice::LocalGemma12B],
            (ModelChoice::LocalGemma270M, _) => {
                vec![ModelChoice::LocalGemma4B, ModelChoice::GeminiFlash]
            }
            (ModelChoice::DeepSeekV32, _) => {
                vec![
                    ModelChoice::DeepSeekV32Speciale,
                    ModelChoice::ClaudeSonnet,
                    ModelChoice::LocalDeepSeekCoder,
                ]
            }
            (ModelChoice::DeepSeekV32Speciale, _) => {
                vec![
                    ModelChoice::DeepSeekV32,
                    ModelChoice::ClaudeOpus,
                    ModelChoice::GeminiPro,
                ]
            }
            // GLM-5.2 steps down to the other low-cost cloud arms, then a
            // capable local model, so a missing GLM key never strands a task.
            (ModelChoice::Glm52, _) => {
                vec![
                    ModelChoice::DeepSeekV32,
                    ModelChoice::GeminiFlash,
                    ModelChoice::LocalMinistral8B,
                ]
            }
            _ => vec![ModelChoice::GeminiFlash],
        }
    }

    fn estimate_cost(model: &ModelChoice, category: TaskCategory) -> f64 {
        let token_estimate = category.estimated_tokens();

        match model {
            ModelChoice::GeminiFlash => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 0.30;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 2.50;
                input_cost + output_cost
            }
            ModelChoice::Gemini35Flash
            | ModelChoice::Gemini35FlashMinimal
            | ModelChoice::Gemini35FlashMedium
            | ModelChoice::Gemini35FlashHigh => {
                // Gemini 3.5 Flash (May 2026): $1.50/M input, $9.00/M output —
                // roughly 3x Gemini 3 Flash ($0.50 / $3.00). Independent Artificial
                // Analysis reporting also flags 5.5x higher token usage when
                // dynamic thinking is on (the server default), so any ARP budget
                // policy migrated from `gemini-3-flash` priors will undercount
                // real spend by ~5–15x for "hard" decisions.
                //
                // We expose distinct ModelChoice arms per thinking tier; the
                // category-based estimate here only knows about visible output
                // tokens. Each arm gets a thinking-multiplier on output to bake
                // in the expected chain-of-thought spend so Thompson Sampling
                // can see the cost difference per arm. Real spend should always
                // come from `usageMetadata` once it's available.
                let thinking_multiplier = match model {
                    ModelChoice::Gemini35FlashMinimal => 0.0,
                    ModelChoice::Gemini35Flash => 0.5, // ~512 tokens / typical 1k output
                    ModelChoice::Gemini35FlashMedium => 2.0,
                    ModelChoice::Gemini35FlashHigh => 8.0,
                    _ => 0.0,
                };
                let effective_output = (token_estimate.output as f64) * (1.0 + thinking_multiplier);
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 1.50;
                let output_cost = (effective_output / 1_000_000.0) * 9.00;
                input_cost + output_cost
            }
            ModelChoice::GeminiPro => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 1.25;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 5.00;
                input_cost + output_cost
            }
            ModelChoice::ClaudeSonnet => {
                // Claude Sonnet 4.5: $3/1M input, $15/1M output
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 3.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 15.00;
                input_cost + output_cost
            }
            ModelChoice::ClaudeOpus => {
                // Claude Opus 4.8: $5/1M input, $25/1M output. The old
                // $15/$75 figure here was Opus 4.1 pricing and overstated
                // Opus cost 3x in every routing comparison.
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 5.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 25.00;
                input_cost + output_cost
            }
            ModelChoice::ClaudeFable5 => {
                // Claude Fable 5: $10/1M input, $50/1M output — 2x Opus.
                // Premium capability tier; cost-justified only on escalation
                // or explicit selection.
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 10.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 50.00;
                input_cost + output_cost
            }
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => {
                // DeepSeek V3.2: $0.27/1M input, $1.10/1M output (cache miss)
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 0.27;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 1.10;
                input_cost + output_cost
            }
            ModelChoice::KimiK2 => {
                // Kimi K2.5: $0.55/1M input, $2.20/1M output
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 0.55;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 2.20;
                input_cost + output_cost
            }
            ModelChoice::Glm52 => {
                // GLM-5.2 (Zhipu AI / Z.ai) published list rates:
                // $1.40/1M input, $4.40/1M output (docs.z.ai/guides/overview/pricing,
                // cross-checked on OpenRouter). The earlier $0.60/$2.20 placeholder
                // was GLM-4.6's rate and undercounted spend ~2x — enough that a
                // single category-sized call rounded to $0.00 in the integer-cent
                // `TokenCost`, so the budget gate treated GLM calls as free. At the
                // real rate a call carries non-zero cost and enforcement engages.
                // Cheap relative to the Anthropic/Gemini tiers, so it stays routable
                // where DeepSeek/Kimi sit. Real spend should come from the response
                // `usage` block once surfaced.
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 1.40;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 4.40;
                input_cost + output_cost
            }
            // All local models are free
            ModelChoice::LocalQwen3
            | ModelChoice::LocalMinistral3B
            | ModelChoice::LocalMinistral8B
            | ModelChoice::LocalQwen35_9B
            | ModelChoice::LocalQwen35_27B
            | ModelChoice::LocalQwen36A3B
            | ModelChoice::LocalGlm47Flash
            | ModelChoice::LocalGemma4E2B
            | ModelChoice::LocalGemma4E4B
            | ModelChoice::LocalGemma4_26B
            | ModelChoice::LocalGemma4_31B
            | ModelChoice::LocalGemma4_12B
            | ModelChoice::LocalGemma270M
            | ModelChoice::LocalGemma4B
            | ModelChoice::LocalGemma12B
            | ModelChoice::LocalDeepSeekCoder => 0.0,
        }
    }

    fn estimate_time(model: &ModelChoice, _category: TaskCategory) -> Duration {
        match model {
            ModelChoice::GeminiFlash => Duration::from_secs(3),
            // Gemini 3.5 Flash sustains ~280 tok/s — faster than legacy Flash.
            // Latency scales roughly with the thinking budget; tiers are
            // hand-tuned from the rimworld run + AA reference numbers.
            ModelChoice::Gemini35FlashMinimal => Duration::from_secs(1),
            ModelChoice::Gemini35Flash => Duration::from_secs(2),
            ModelChoice::Gemini35FlashMedium => Duration::from_secs(6),
            ModelChoice::Gemini35FlashHigh => Duration::from_secs(20),
            ModelChoice::GeminiPro => Duration::from_secs(10),
            ModelChoice::ClaudeSonnet => Duration::from_secs(5),
            // Opus 4.8 and Fable 5 both reason via adaptive thinking, which
            // spends extra wall-clock on hard tasks.
            ModelChoice::ClaudeOpus => Duration::from_secs(18),
            ModelChoice::ClaudeFable5 => Duration::from_secs(20),
            ModelChoice::LocalQwen3 => Duration::from_millis(500),
            ModelChoice::LocalMinistral3B => Duration::from_secs(2),
            ModelChoice::LocalMinistral8B => Duration::from_secs(4),
            ModelChoice::LocalQwen35_9B => Duration::from_secs(4),
            ModelChoice::LocalQwen35_27B => Duration::from_secs(10),
            ModelChoice::LocalQwen36A3B => Duration::from_secs(6),
            ModelChoice::LocalGlm47Flash => Duration::from_secs(8),
            ModelChoice::LocalGemma4E2B => Duration::from_secs(1),
            ModelChoice::LocalGemma4E4B => Duration::from_secs(2),
            ModelChoice::LocalGemma4_26B => Duration::from_secs(8),
            ModelChoice::LocalGemma4_31B => Duration::from_secs(12),
            ModelChoice::LocalGemma4_12B => Duration::from_secs(6),
            ModelChoice::LocalGemma270M => Duration::from_millis(500),
            ModelChoice::LocalGemma4B => Duration::from_secs(2),
            ModelChoice::LocalGemma12B => Duration::from_secs(5),
            ModelChoice::LocalDeepSeekCoder => Duration::from_secs(4),
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => Duration::from_secs(5),
            ModelChoice::KimiK2 => Duration::from_secs(5),
            // GLM-5.2 reasons before answering; budget extra wall-clock on
            // hard tasks, in line with the other thinking cloud arms.
            ModelChoice::Glm52 => Duration::from_secs(8),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TokenEstimate {
    pub input: u32,
    pub output: u32,
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use arkavo_budget::TokenCost;
    use arkavo_budget::config::{BudgetConfig, BudgetLimits};
    use arkavo_budget::cost::TokenUsage;
    use arkavo_budget::tracker::BudgetTracker;
    use arkavo_test_macros::spec;

    #[test]
    fn test_model_choice_name() {
        assert_eq!(ModelChoice::GeminiFlash.name(), "gemini-flash-latest");
        assert_eq!(ModelChoice::Gemini35Flash.name(), "gemini-3.5-flash");
        assert_eq!(ModelChoice::LocalGemma270M.name(), "gemma-3-270m-it");
    }

    // Regression: ClaudeOpus must pin the current dateless Opus snapshot.
    // The provider routes `claude-opus-4-8` onto the adaptive-thinking surface
    // (no sampling params), so a stale dated id here would also send the wrong
    // request shape.
    #[test]
    fn test_claude_opus_pins_opus_4_8() {
        assert_eq!(ModelChoice::ClaudeOpus.name(), "claude-opus-4-8");
        assert_eq!(
            ModelChoice::from_name("claude-opus-4-8"),
            Some(ModelChoice::ClaudeOpus)
        );
    }

    #[test]
    fn test_claude_fable_5_properties() {
        let model = ModelChoice::ClaudeFable5;
        assert_eq!(model.name(), "claude-fable-5");
        assert_eq!(model.provider(), "anthropic");
        assert_eq!(model.family(), "anthropic");
        assert_eq!(model.display_name(), "Claude Fable 5");
        assert!(model.is_anthropic());
        assert!(model.is_cloud());
        assert!(!model.is_local());
        assert_eq!(model.capability(), PlannerTier::Large);
    }

    #[test]
    fn test_claude_fable_5_name_resolution() {
        for alias in ["claude-fable-5", "claude-fable", "fable"] {
            assert_eq!(
                ModelChoice::from_name(alias),
                Some(ModelChoice::ClaudeFable5),
                "alias {alias} should resolve to ClaudeFable5"
            );
        }
        assert_eq!(
            ModelChoice::from_name(ModelChoice::ClaudeFable5.name()),
            Some(ModelChoice::ClaudeFable5),
            "round-trip via primary name"
        );
    }

    #[test]
    fn test_claude_fable_5_costs_double_opus() {
        // Fable 5 is $10/$50 vs Opus 4.8 at $5/$25 — the premium tier must
        // estimate at exactly 2x Opus so budget policies see the real spread.
        let fable = RoutingDecision::estimate_cost(
            &ModelChoice::ClaudeFable5,
            TaskCategory::CodeGeneration,
        );
        let opus =
            RoutingDecision::estimate_cost(&ModelChoice::ClaudeOpus, TaskCategory::CodeGeneration);
        assert!(fable > 0.0);
        assert!(
            (fable - opus * 2.0).abs() < 1e-9,
            "fable={fable} opus={opus}"
        );
    }

    // Regression: ClaudeOpus pins claude-opus-4-8, which is $5/$25 per MTok.
    // The previous $15/$75 figure (Opus 4.1 pricing) overstated cost 3x.
    #[test]
    fn test_claude_opus_cost_uses_opus_4_8_pricing() {
        let opus =
            RoutingDecision::estimate_cost(&ModelChoice::ClaudeOpus, TaskCategory::CodeGeneration);
        let tokens = TaskCategory::CodeGeneration.estimated_tokens();
        let expected = (f64::from(tokens.input) / 1_000_000.0) * 5.00
            + (f64::from(tokens.output) / 1_000_000.0) * 25.00;
        assert!((opus - expected).abs() < 1e-9);
    }

    #[test]
    fn test_claude_fable_5_fallback_chain_steps_down_to_opus() {
        let decision = RoutingDecision::new(
            ModelChoice::ClaudeFable5,
            TaskCategory::CodeGeneration,
            0.9,
            "Test".to_string(),
        );
        assert_eq!(
            decision.fallback_chain.first(),
            Some(&ModelChoice::ClaudeOpus)
        );
        assert!(decision.fallback_chain.iter().any(ModelChoice::is_local));
    }

    #[test]
    fn test_gemini_3_5_flash_properties() {
        let model = ModelChoice::Gemini35Flash;
        assert_eq!(model.provider(), "google");
        assert_eq!(model.family(), "google");
        assert_eq!(model.display_name(), "Gemini 3.5 Flash (low)");
        assert!(model.is_gemini());
        assert!(model.is_cloud());
        assert!(!model.is_local());
        assert_eq!(model.capability(), PlannerTier::Large);
    }

    #[test]
    fn test_gemini_3_5_flash_aliases() {
        for alias in [
            "gemini-3.5-flash",
            "gemini-3.5-flash-latest",
            "gemini-3.5",
            "gemini35flash",
            "gemini-3.1-flash-live",
            "gemini-3.1-flash-tts",
        ] {
            assert_eq!(
                ModelChoice::from_name(alias),
                Some(ModelChoice::Gemini35Flash),
                "alias {alias} should resolve to Gemini35Flash"
            );
        }
        assert_eq!(
            ModelChoice::from_name(ModelChoice::Gemini35Flash.name()),
            Some(ModelChoice::Gemini35Flash),
            "round-trip via primary name"
        );
    }

    #[test]
    fn test_gemini_3_5_flash_cost_matches_announcement() {
        // Gemini 3.5 Flash (May 2026): $1.50/M input, $9.00/M output —
        // priced above legacy Flash and above Pro on output, reflecting the
        // thinking/agentic upgrade noted in the launch announcement.
        let cost = RoutingDecision::estimate_cost(
            &ModelChoice::Gemini35Flash,
            TaskCategory::CodeGeneration,
        );
        let flash_cost =
            RoutingDecision::estimate_cost(&ModelChoice::GeminiFlash, TaskCategory::CodeGeneration);
        let pro_cost =
            RoutingDecision::estimate_cost(&ModelChoice::GeminiPro, TaskCategory::CodeGeneration);
        assert!(cost > 0.0, "Gemini 3.5 Flash should be priced");
        assert!(
            cost > flash_cost,
            "3.5 Flash should cost more than legacy Flash"
        );
        assert!(
            cost > pro_cost,
            "3.5 Flash output is $9/M vs Pro $5/M, so total exceeds Pro at this token mix"
        );
    }

    #[test]
    fn test_model_choice_is_local() {
        assert!(ModelChoice::LocalGemma4B.is_local());
        assert!(!ModelChoice::GeminiFlash.is_local());
    }

    #[test]
    fn test_routing_decision_cost() {
        let decision = RoutingDecision::new(
            ModelChoice::LocalGemma4B,
            TaskCategory::CodeSearch,
            0.9,
            "Local model for code search".to_string(),
        );

        assert_eq!(decision.estimated_cost_usd, 0.0);
    }

    #[test]
    fn test_glm52_properties() {
        let model = ModelChoice::Glm52;
        assert_eq!(model.name(), "glm-5.2");
        assert_eq!(model.provider(), "zhipu");
        assert_eq!(model.family(), "glm");
        assert_eq!(model.display_name(), "GLM-5.2");
        assert!(model.is_glm());
        assert!(model.is_cloud());
        assert!(!model.is_local());
        assert_eq!(model.capability(), PlannerTier::Large);
    }

    #[test]
    fn test_glm52_name_resolution() {
        for alias in ["glm-5.2", "glm5.2", "glm-5", "glm52", "GLM-5.2"] {
            assert_eq!(
                ModelChoice::from_name(alias),
                Some(ModelChoice::Glm52),
                "alias {alias} should resolve to Glm52"
            );
        }
        assert_eq!(
            ModelChoice::from_name(ModelChoice::Glm52.name()),
            Some(ModelChoice::Glm52),
            "round-trip via primary name"
        );
        // GLM-5.2 (cloud) must not collide with the local GLM-4.7-Flash GGUF.
        assert_eq!(
            ModelChoice::from_name("glm-4.7-flash"),
            Some(ModelChoice::LocalGlm47Flash)
        );
    }

    // BUDGET-001 (track token cost): the cost-calculation half — a GLM call is
    // priced from real provider rates, the precondition for any budget tracking.
    #[spec("BUDGET-001")]
    #[test]
    fn test_glm52_cost_is_priced_below_anthropic() {
        // Budget enforcement only engages if GLM has a real, non-zero cost
        // model. GLM-5.2 must be priced (cloud) yet sit well under Opus.
        let glm = RoutingDecision::estimate_cost(&ModelChoice::Glm52, TaskCategory::CodeGeneration);
        let opus =
            RoutingDecision::estimate_cost(&ModelChoice::ClaudeOpus, TaskCategory::CodeGeneration);
        assert!(
            glm > 0.0,
            "GLM-5.2 must carry a real cost for budget gating"
        );
        assert!(glm < opus, "GLM-5.2 should be cheaper than Opus");
    }

    #[test]
    fn test_glm52_fallback_chain_has_local_safety_net() {
        let decision = RoutingDecision::new(
            ModelChoice::Glm52,
            TaskCategory::CodeGeneration,
            0.9,
            "Test".to_string(),
        );
        assert!(decision.fallback_chain.iter().any(ModelChoice::is_local));
    }

    // A realistic GLM-5.2 call (200K input + 100K output) priced at the
    // `ModelChoice::Glm52` arm's published rates ($1.40 / $4.40 per 1M). Kept in
    // one place so these budget tests track the cost arm above.
    fn glm52_call_cost() -> TokenCost {
        let usd = (200_000.0 / 1_000_000.0) * 1.40 + (100_000.0 / 1_000_000.0) * 4.40;
        TokenCost::from_dollars(usd)
    }

    // BUDGET-001 (track token cost): a real routed GLM call accrues non-zero,
    // tracked spend. This exercises the exact orchestrator conversion
    // (`TokenCost::from_dollars(decision.estimated_cost_usd)`), so it also guards
    // the integer-cent flooring regression — under the old $0.60/$2.20 placeholder
    // a category-sized call rounded to $0.00 and escaped tracking entirely.
    #[spec("BUDGET-001")]
    #[tokio::test]
    async fn test_glm52_call_accrues_tracked_cost() {
        let decision = RoutingDecision::new(
            ModelChoice::Glm52,
            TaskCategory::CodeGeneration,
            0.9,
            "Generate code via GLM".to_string(),
        );
        let cost = TokenCost::from_dollars(decision.estimated_cost_usd);
        assert!(
            cost > TokenCost::ZERO,
            "a routed GLM call must price above zero cents or budget tracking is a no-op"
        );

        let tracker = BudgetTracker::new(BudgetConfig::default()).await.unwrap();
        tracker
            .record_spending(
                "agent-glm".to_string(),
                ModelChoice::Glm52.provider().to_string(),
                ModelChoice::Glm52.name().to_string(),
                TokenUsage::new(800, 3000),
                cost,
            )
            .await
            .unwrap();

        assert_eq!(
            tracker.get_status().await.session_spent,
            cost,
            "running total must reflect the GLM call's cost"
        );
    }

    // BUDGET-002 (enforce budget limit before call): with spend near the cap, a
    // GLM call is rejected and nothing is recorded — proving the paid arm is
    // bounded.
    #[spec("BUDGET-002")]
    #[tokio::test]
    async fn test_glm52_call_blocked_when_over_budget() {
        let config = BudgetConfig {
            limits: BudgetLimits {
                session_limit: Some(TokenCost::from_dollars(1.00)),
                hourly_limit: None,
                daily_limit: None,
                monthly_limit: None,
                total_limit: None,
            },
            ..Default::default()
        };
        let tracker = BudgetTracker::new(config).await.unwrap();
        // Current spend near the $1.00 session cap.
        tracker
            .record_spending(
                "agent-glm".to_string(),
                ModelChoice::Glm52.provider().to_string(),
                ModelChoice::Glm52.name().to_string(),
                TokenUsage::new(0, 0),
                TokenCost::from_dollars(0.99),
            )
            .await
            .unwrap();

        let call = glm52_call_cost();
        assert!(
            !tracker.can_afford("agent-glm", call).await.unwrap(),
            "GLM call that overruns the cap must not be affordable"
        );
        assert!(
            tracker
                .try_spend(
                    "agent-glm".to_string(),
                    ModelChoice::Glm52.provider().to_string(),
                    ModelChoice::Glm52.name().to_string(),
                    TokenUsage::new(200_000, 100_000),
                    call,
                )
                .await
                .is_err(),
            "try_spend must error rather than dispatch an over-budget GLM call"
        );
        assert_eq!(
            tracker.get_status().await.session_spent,
            TokenCost::from_dollars(0.99),
            "a blocked GLM call must not accrue spend"
        );
    }

    // BUDGET-002 (enforce budget limit before call): the gate is bidirectional —
    // the same GLM call clears and records when it fits under the budget.
    #[spec("BUDGET-002")]
    #[tokio::test]
    async fn test_glm52_call_allowed_within_budget() {
        let tracker = BudgetTracker::new(BudgetConfig::default()).await.unwrap();
        let call = glm52_call_cost();
        assert!(
            tracker.can_afford("agent-glm", call).await.unwrap(),
            "GLM call within the default budget must be affordable"
        );
        tracker
            .try_spend(
                "agent-glm".to_string(),
                ModelChoice::Glm52.provider().to_string(),
                ModelChoice::Glm52.name().to_string(),
                TokenUsage::new(200_000, 100_000),
                call,
            )
            .await
            .expect("an affordable GLM call must record spend");
        assert_eq!(tracker.get_status().await.session_spent, call);
    }

    #[test]
    fn test_deepseek_model_properties() {
        assert_eq!(ModelChoice::DeepSeekV32.name(), "deepseek-chat");
        assert_eq!(ModelChoice::DeepSeekV32Speciale.name(), "deepseek-chat");
        assert_eq!(ModelChoice::DeepSeekV32.provider(), "deepseek");
        assert_eq!(ModelChoice::DeepSeekV32Speciale.provider(), "deepseek");
        assert!(ModelChoice::DeepSeekV32.is_deepseek());
        assert!(ModelChoice::DeepSeekV32Speciale.is_deepseek());
        assert!(ModelChoice::DeepSeekV32.is_cloud());
        assert!(!ModelChoice::DeepSeekV32.is_local());
    }

    #[test]
    fn test_model_choice_family() {
        assert_eq!(ModelChoice::LocalQwen3.family(), "qwen");
        assert_eq!(ModelChoice::LocalMinistral3B.family(), "mistral");
        assert_eq!(ModelChoice::LocalMinistral8B.family(), "mistral");
        assert_eq!(ModelChoice::LocalGlm47Flash.family(), "glm");
        assert_eq!(ModelChoice::LocalGemma270M.family(), "gemma");
        assert_eq!(ModelChoice::LocalGemma4B.family(), "gemma");
        assert_eq!(ModelChoice::GeminiFlash.family(), "google");
        assert_eq!(ModelChoice::ClaudeSonnet.family(), "anthropic");
        assert_eq!(ModelChoice::DeepSeekV32.family(), "deepseek");
        assert_eq!(ModelChoice::KimiK2.family(), "kimi");
    }

    #[test]
    fn test_model_choice_from_name() {
        assert_eq!(
            ModelChoice::from_name("qwen3.5-27b"),
            Some(ModelChoice::LocalQwen35_27B)
        );
        assert_eq!(
            ModelChoice::from_name("glm-4.7-flash"),
            Some(ModelChoice::LocalGlm47Flash)
        );
        assert_eq!(
            ModelChoice::from_name("ministral-8b"),
            Some(ModelChoice::LocalMinistral8B)
        );
        assert_eq!(ModelChoice::from_name("unknown-model"), None);
        // Round-trip: name -> from_name -> name
        for model in [
            ModelChoice::LocalQwen3,
            ModelChoice::LocalMinistral3B,
            ModelChoice::LocalQwen35_27B,
            ModelChoice::LocalQwen36A3B,
            ModelChoice::LocalGlm47Flash,
            ModelChoice::KimiK2,
        ] {
            assert_eq!(
                ModelChoice::from_name(model.name()),
                Some(model.clone()),
                "Round-trip failed for {model:?}"
            );
        }
    }

    #[test]
    fn test_model_choice_capability() {
        // Small models (< 2B params)
        assert_eq!(ModelChoice::LocalQwen3.capability(), PlannerTier::Small);
        assert_eq!(ModelChoice::LocalGemma270M.capability(), PlannerTier::Small);

        // Medium models (2-7B params)
        assert_eq!(
            ModelChoice::LocalMinistral3B.capability(),
            PlannerTier::Medium
        );
        assert_eq!(ModelChoice::LocalGemma4B.capability(), PlannerTier::Medium);

        // Large models (> 7B params or cloud)
        assert_eq!(
            ModelChoice::LocalMinistral8B.capability(),
            PlannerTier::Large
        );
        assert_eq!(ModelChoice::GeminiFlash.capability(), PlannerTier::Large);
        assert_eq!(ModelChoice::ClaudeSonnet.capability(), PlannerTier::Large);
    }

    #[test]
    fn test_downgrade_model_fits_budget() {
        // Model fits — no downgrade
        let model = ModelChoice::LocalQwen3; // 550MB
        let result = model.downgrade_for_budget(2 * 1024 * 1024 * 1024); // 2GB budget
        assert_eq!(result, ModelChoice::LocalQwen3);
    }

    #[test]
    fn test_downgrade_model_exceeds_budget() {
        // ministral-3b (2.5GB) exceeds 1GB budget → should downgrade to qwen3.5-0.8b (550MB)
        let model = ModelChoice::LocalMinistral3B;
        let result = model.downgrade_for_budget(1024 * 1024 * 1024); // 1GB budget
        assert_eq!(result, ModelChoice::LocalQwen3); // 550MB fits
    }

    #[test]
    fn test_downgrade_picks_largest_that_fits() {
        // 6GB budget should allow qwen3.5-9b (6GB)
        let model = ModelChoice::LocalQwen35_27B; // 23GB
        let result = model.downgrade_for_budget(6 * 1024 * 1024 * 1024);
        assert_eq!(result, ModelChoice::LocalQwen35_9B);
    }

    #[test]
    fn test_downgrade_cloud_model_not_constrained() {
        // Cloud models have size_bytes() == 0, never downgraded
        let model = ModelChoice::GeminiFlash;
        let result = model.downgrade_for_budget(512 * 1024 * 1024);
        assert_eq!(result, ModelChoice::GeminiFlash);
    }

    #[test]
    fn test_downgrade_zero_budget_no_constraint() {
        // max_bytes=0 means no constraint
        let model = ModelChoice::LocalMinistral3B;
        let result = model.downgrade_for_budget(0);
        assert_eq!(result, ModelChoice::LocalMinistral3B);
    }
}
