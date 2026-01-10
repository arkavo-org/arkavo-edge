use crate::classifier::TaskCategory;
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
    /// Qwen3-0.6B - fast, TØRG-compatible (preferred default)
    LocalQwen3,
    /// Ministral-3B - TØRG-compatible, higher quality
    LocalMinistral3B,
    /// Ministral-8B - TØRG-compatible, high quality
    LocalMinistral8B,
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
}

impl ModelChoice {
    pub fn name(&self) -> &str {
        match self {
            Self::GeminiFlash => "gemini-flash-latest",
            Self::GeminiPro => "gemini-3-pro-preview",
            Self::ClaudeSonnet => "claude-sonnet-4-5-20250929",
            Self::ClaudeOpus => "claude-opus-4-5-20251101",
            Self::LocalQwen3 => "qwen3-0.6b",
            Self::LocalMinistral3B => "ministral-3b",
            Self::LocalMinistral8B => "ministral-8b",
            Self::LocalGemma270M => "gemma-3-270m-it",
            Self::LocalGemma4B => "gemma-3-4b-it",
            Self::LocalGemma12B => "gemma-3-12b-it",
            Self::LocalDeepSeekCoder => "deepseek-coder-v2-lite-instruct",
            Self::DeepSeekV32 | Self::DeepSeekV32Speciale => "deepseek-chat",
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(
            self,
            Self::LocalQwen3
                | Self::LocalMinistral3B
                | Self::LocalMinistral8B
                | Self::LocalGemma270M
                | Self::LocalGemma4B
                | Self::LocalGemma12B
                | Self::LocalDeepSeekCoder
        )
    }

    pub fn is_cloud(&self) -> bool {
        !self.is_local()
    }

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

    pub fn provider(&self) -> &str {
        match self {
            Self::GeminiFlash | Self::GeminiPro => "google",
            Self::ClaudeSonnet | Self::ClaudeOpus => "anthropic",
            Self::LocalQwen3 => "local-qwen",
            Self::LocalMinistral3B | Self::LocalMinistral8B => "local-ministral",
            Self::LocalGemma270M | Self::LocalGemma4B | Self::LocalGemma12B => "local-gemma",
            Self::LocalDeepSeekCoder => "local-deepseek",
            Self::DeepSeekV32 | Self::DeepSeekV32Speciale => "deepseek",
        }
    }

    /// Get the capability tier for this model based on parameter count
    pub fn capability(&self) -> PlannerTier {
        match self {
            // Small: < 2B parameters
            Self::LocalQwen3 | Self::LocalGemma270M => PlannerTier::Small,
            // Medium: 2-7B parameters
            Self::LocalMinistral3B | Self::LocalGemma4B => PlannerTier::Medium,
            // Large: > 7B parameters or cloud models
            Self::LocalMinistral8B
            | Self::LocalGemma12B
            | Self::LocalDeepSeekCoder
            | Self::GeminiFlash
            | Self::GeminiPro
            | Self::ClaudeSonnet
            | Self::ClaudeOpus
            | Self::DeepSeekV32
            | Self::DeepSeekV32Speciale => PlannerTier::Large,
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
}

impl RoutingDecision {
    pub fn new(
        model: ModelChoice,
        category: TaskCategory,
        confidence: f32,
        reasoning: String,
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
            // Qwen3 -> Ministral-3B -> Ministral-8B
            (ModelChoice::LocalQwen3, _) => {
                vec![ModelChoice::LocalMinistral3B, ModelChoice::GeminiFlash]
            }
            (ModelChoice::LocalMinistral3B, _) => vec![ModelChoice::LocalMinistral8B],
            (ModelChoice::LocalMinistral8B, _) => vec![ModelChoice::GeminiFlash],
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
            // All local models are free
            ModelChoice::LocalQwen3
            | ModelChoice::LocalMinistral3B
            | ModelChoice::LocalMinistral8B
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
            ModelChoice::LocalGemma270M => Duration::from_millis(500),
            ModelChoice::LocalGemma4B => Duration::from_secs(2),
            ModelChoice::LocalGemma12B => Duration::from_secs(5),
            ModelChoice::LocalDeepSeekCoder => Duration::from_secs(4),
            ModelChoice::DeepSeekV32 | ModelChoice::DeepSeekV32Speciale => Duration::from_secs(5),
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
    fn test_model_choice_capability() {
        // Small models (< 2B params)
        assert_eq!(ModelChoice::LocalQwen3.capability(), PlannerTier::Small);
        assert_eq!(ModelChoice::LocalGemma270M.capability(), PlannerTier::Small);

        // Medium models (2-7B params)
        assert_eq!(ModelChoice::LocalMinistral3B.capability(), PlannerTier::Medium);
        assert_eq!(ModelChoice::LocalGemma4B.capability(), PlannerTier::Medium);

        // Large models (> 7B params or cloud)
        assert_eq!(ModelChoice::LocalMinistral8B.capability(), PlannerTier::Large);
        assert_eq!(ModelChoice::GeminiFlash.capability(), PlannerTier::Large);
        assert_eq!(ModelChoice::ClaudeSonnet.capability(), PlannerTier::Large);
    }
}
