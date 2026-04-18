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
    GeminiPro,
    ClaudeSonnet,
    ClaudeOpus,
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
}

impl ModelChoice {
    pub fn name(&self) -> &str {
        match self {
            Self::GeminiFlash => "gemini-flash-latest",
            Self::GeminiPro => "gemini-3-pro-preview",
            Self::ClaudeSonnet => "claude-sonnet-4-5-20250929",
            Self::ClaudeOpus => "claude-opus-4-5-20251101",
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
            Self::LocalGemma270M => "gemma-3-270m-it",
            Self::LocalGemma4B => "gemma-3-4b-it",
            Self::LocalGemma12B => "gemma-3-12b-it",
            Self::LocalDeepSeekCoder => "deepseek-coder-v2-lite-instruct",
            Self::DeepSeekV32 | Self::DeepSeekV32Speciale => "deepseek-chat",
            Self::KimiK2 => "kimi-k2.5",
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
            Self::LocalGlm47Flash => "glm",
            Self::LocalGemma4E2B
            | Self::LocalGemma4E4B
            | Self::LocalGemma4_26B
            | Self::LocalGemma4_31B
            | Self::LocalGemma270M
            | Self::LocalGemma4B
            | Self::LocalGemma12B => "gemma",
            Self::LocalDeepSeekCoder | Self::DeepSeekV32 | Self::DeepSeekV32Speciale => "deepseek",
            Self::GeminiFlash | Self::GeminiPro => "google",
            Self::ClaudeSonnet | Self::ClaudeOpus => "anthropic",
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
            "gemma-4-e2b" => Some(Self::LocalGemma4E2B),
            "gemma-4-e4b" => Some(Self::LocalGemma4E4B),
            "gemma-4-26b-a4b" => Some(Self::LocalGemma4_26B),
            "gemma-4-31b" => Some(Self::LocalGemma4_31B),
            "gemma-3-270m-it" => Some(Self::LocalGemma270M),
            "gemma-3-4b-it" => Some(Self::LocalGemma4B),
            "gemma-3-12b-it" => Some(Self::LocalGemma12B),
            "deepseek-coder-v2-lite-instruct" => Some(Self::LocalDeepSeekCoder),
            "deepseek-chat" => Some(Self::DeepSeekV32),
            "kimi-k2.5" => Some(Self::KimiK2),
            "gemini-flash-latest" => Some(Self::GeminiFlash),
            "gemini-3-pro-preview" => Some(Self::GeminiPro),
            _ if name.contains("claude-sonnet") => Some(Self::ClaudeSonnet),
            _ if name.contains("claude-opus") => Some(Self::ClaudeOpus),
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
        Self::LocalDeepSeekCoder,
        Self::LocalGemma4_26B,
        Self::LocalQwen35_27B,
        Self::LocalGlm47Flash,
        Self::LocalQwen36A3B,
        Self::LocalGemma4_31B,
    ];

    pub fn is_anthropic(&self) -> bool {
        matches!(self, Self::ClaudeSonnet | Self::ClaudeOpus)
    }

    pub fn is_gemini(&self) -> bool {
        matches!(self, Self::GeminiFlash | Self::GeminiPro)
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

    pub fn provider(&self) -> &str {
        match self {
            Self::GeminiFlash | Self::GeminiPro => "google",
            Self::ClaudeSonnet | Self::ClaudeOpus => "anthropic",
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
            | Self::LocalGemma270M
            | Self::LocalGemma4B
            | Self::LocalGemma12B => "local-gemma",
            Self::LocalDeepSeekCoder => "local-deepseek",
            Self::DeepSeekV32 | Self::DeepSeekV32Speciale => "deepseek",
            Self::KimiK2 => "kimi",
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
            | Self::LocalMinistral8B
            | Self::LocalQwen35_9B
            | Self::LocalQwen35_27B
            | Self::LocalQwen36A3B
            | Self::LocalGlm47Flash
            | Self::LocalGemma12B
            | Self::LocalDeepSeekCoder
            | Self::GeminiFlash
            | Self::GeminiPro
            | Self::ClaudeSonnet
            | Self::ClaudeOpus
            | Self::DeepSeekV32
            | Self::DeepSeekV32Speciale
            | Self::KimiK2 => PlannerTier::Large,
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
            Self::LocalQwen36A3B,
            Self::LocalGlm47Flash,
            Self::LocalQwen35_27B,
            Self::LocalDeepSeekCoder,
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
            Self::LocalGemma270M => "Gemma 270M",
            Self::LocalGemma4B => "Gemma 4B",
            Self::LocalGemma12B => "Gemma 12B",
            Self::LocalDeepSeekCoder => "DeepSeek Coder",
            Self::GeminiFlash => "Gemini Flash",
            Self::GeminiPro => "Gemini Pro",
            Self::ClaudeSonnet => "Claude Sonnet",
            Self::ClaudeOpus => "Claude Opus",
            Self::DeepSeekV32 => "DeepSeek V3.2",
            Self::DeepSeekV32Speciale => "DeepSeek V3.2 Speciale",
            Self::KimiK2 => "Kimi K2.5",
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
                    ModelChoice::GeminiPro,
                    ModelChoice::ClaudeSonnet,
                    ModelChoice::LocalMinistral3B,
                ]
            }
            (ModelChoice::GeminiPro, _) => {
                vec![
                    ModelChoice::ClaudeOpus,
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
                // Claude Opus 4.5: $15/1M input, $75/1M output
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 15.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 75.00;
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
            | ModelChoice::LocalGemma270M
            | ModelChoice::LocalGemma4B
            | ModelChoice::LocalGemma12B
            | ModelChoice::LocalDeepSeekCoder => 0.0,
        }
    }

    fn estimate_time(model: &ModelChoice, _category: TaskCategory) -> Duration {
        match model {
            ModelChoice::GeminiFlash => Duration::from_secs(3),
            ModelChoice::GeminiPro => Duration::from_secs(10),
            ModelChoice::ClaudeSonnet => Duration::from_secs(5),
            ModelChoice::ClaudeOpus => Duration::from_secs(15),
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
            ModelChoice::LocalGemma270M => Duration::from_millis(500),
            ModelChoice::LocalGemma4B => Duration::from_secs(2),
            ModelChoice::LocalGemma12B => Duration::from_secs(5),
            ModelChoice::LocalDeepSeekCoder => Duration::from_secs(4),
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => Duration::from_secs(5),
            ModelChoice::KimiK2 => Duration::from_secs(5),
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

    #[test]
    fn test_model_choice_name() {
        assert_eq!(ModelChoice::GeminiFlash.name(), "gemini-flash-latest");
        assert_eq!(ModelChoice::LocalGemma270M.name(), "gemma-3-270m-it");
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
                "Round-trip failed for {:?}",
                model
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
