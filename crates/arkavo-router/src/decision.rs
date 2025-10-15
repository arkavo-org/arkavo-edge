use crate::classifier::TaskCategory;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ModelChoice {
    GeminiFlash,
    GeminiPro,
    LocalGemma270M,
    LocalGemma4B,
    LocalGemma12B,
}

impl ModelChoice {
    pub fn name(&self) -> &str {
        match self {
            Self::GeminiFlash => "gemini-flash-latest",
            Self::GeminiPro => "gemini-2.5-pro",
            Self::LocalGemma270M => "gemma-3-270m-it",
            Self::LocalGemma4B => "gemma-3-4b-it",
            Self::LocalGemma12B => "gemma-3-12b-it",
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(
            self,
            Self::LocalGemma270M | Self::LocalGemma4B | Self::LocalGemma12B
        )
    }

    pub fn is_cloud(&self) -> bool {
        !self.is_local()
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
                vec![ModelChoice::GeminiPro, ModelChoice::LocalGemma4B]
            }
            (ModelChoice::GeminiPro, _) => {
                vec![ModelChoice::GeminiFlash, ModelChoice::LocalGemma12B]
            }
            (ModelChoice::LocalGemma4B, _) => vec![ModelChoice::LocalGemma12B],
            (ModelChoice::LocalGemma270M, _) => {
                vec![ModelChoice::LocalGemma4B, ModelChoice::GeminiFlash]
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
            ModelChoice::LocalGemma270M
            | ModelChoice::LocalGemma4B
            | ModelChoice::LocalGemma12B => 0.0,
        }
    }

    fn estimate_time(model: &ModelChoice, _category: TaskCategory) -> Duration {
        match model {
            ModelChoice::GeminiFlash => Duration::from_secs(3),
            ModelChoice::GeminiPro => Duration::from_secs(10),
            ModelChoice::LocalGemma270M => Duration::from_millis(500),
            ModelChoice::LocalGemma4B => Duration::from_secs(2),
            ModelChoice::LocalGemma12B => Duration::from_secs(5),
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
}
