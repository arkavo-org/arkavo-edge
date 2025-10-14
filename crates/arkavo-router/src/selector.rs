use crate::Result;
use crate::classifier::{Classification, TaskCategory};
use crate::decision::{ModelChoice, RoutingDecision};

pub struct ModelSelector {
    budget_threshold: f64,
}

impl ModelSelector {
    pub fn new() -> Self {
        Self {
            budget_threshold: 0.80,
        }
    }

    pub fn with_budget_threshold(budget_threshold: f64) -> Self {
        Self { budget_threshold }
    }

    pub fn select(
        &self,
        classification: &Classification,
        _task_description: &str,
    ) -> Result<RoutingDecision> {
        let model = self.select_model_by_category(classification);

        let reasoning = self.explain_selection(&model, classification);

        Ok(RoutingDecision::new(
            model,
            classification.category,
            classification.confidence,
            reasoning,
        ))
    }

    fn select_model_by_category(&self, classification: &Classification) -> ModelChoice {
        match classification.category {
            TaskCategory::FrontendUI if classification.confidence > 0.75 => {
                ModelChoice::GeminiFlash
            }

            TaskCategory::BackendAPI if classification.confidence > 0.70 => ModelChoice::GeminiPro,

            TaskCategory::CodeSearch => ModelChoice::LocalGemma4B,

            TaskCategory::SecurityScan => ModelChoice::LocalGemma4B,

            TaskCategory::TestGeneration if classification.confidence > 0.70 => {
                ModelChoice::GeminiPro
            }

            TaskCategory::Documentation => ModelChoice::LocalGemma4B,

            TaskCategory::Refactoring if classification.confidence > 0.75 => {
                ModelChoice::GeminiFlash
            }

            TaskCategory::VisionAnalysis => ModelChoice::GeminiFlash,

            _ => ModelChoice::GeminiFlash,
        }
    }

    fn explain_selection(&self, model: &ModelChoice, classification: &Classification) -> String {
        let category_reason = match classification.category {
            TaskCategory::FrontendUI => "Frontend task: Gemini Flash ranks #1 on WebDev Arena",
            TaskCategory::BackendAPI => "Backend API: Gemini Pro provides highest quality",
            TaskCategory::CodeSearch => "Code search: Local Gemma 4B is fast and free",
            TaskCategory::SecurityScan => "Security scan: Local Gemma 4B for privacy",
            TaskCategory::TestGeneration => "Test generation: Gemini Pro for comprehensive tests",
            TaskCategory::Documentation => "Documentation: Local Gemma 4B sufficient",
            TaskCategory::Refactoring => "Refactoring: Gemini Flash for quick iterations",
            TaskCategory::VisionAnalysis => "Vision analysis: Gemini Flash with multimodal support",
            TaskCategory::General => "General task: Gemini Flash as balanced default",
        };

        let model_benefit = match model {
            ModelChoice::GeminiFlash => "Fast (3s), cost-effective ($0.003-0.006)",
            ModelChoice::GeminiPro => "Highest quality, comprehensive output ($0.009)",
            ModelChoice::LocalGemma270M => "Ultra-fast (<1s), zero cost",
            ModelChoice::LocalGemma4B => "Fast (2s), zero cost, private",
            ModelChoice::LocalGemma12B => "High quality, zero cost, private",
        };

        format!(
            "{}. Using {}: {}",
            category_reason,
            model.name(),
            model_benefit
        )
    }

    pub async fn select_with_budget_constraint(
        &self,
        classification: &Classification,
        task_description: &str,
        budget_usage_percent: f64,
    ) -> Result<RoutingDecision> {
        let mut decision = self.select(classification, task_description)?;

        if budget_usage_percent > self.budget_threshold && decision.recommended_model.is_cloud() {
            let local_fallback = match classification.category {
                TaskCategory::FrontendUI | TaskCategory::BackendAPI | TaskCategory::Refactoring => {
                    ModelChoice::LocalGemma12B
                }
                _ => ModelChoice::LocalGemma4B,
            };

            decision.reasoning = format!(
                "Budget constrained ({}% used). Switching to local model: {}. Original: {}",
                (budget_usage_percent * 100.0) as u32,
                local_fallback.name(),
                decision.reasoning
            );

            decision.recommended_model = local_fallback;
            decision.estimated_cost_usd = 0.0;
        }

        Ok(decision)
    }
}

impl Default for ModelSelector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_frontend_routing() {
        let selector = ModelSelector::new();

        let classification = Classification {
            category: TaskCategory::FrontendUI,
            confidence: 0.90,
            reasoning: "Frontend task".to_string(),
        };

        let decision = selector
            .select(&classification, "Build a React component")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::GeminiFlash);
        assert!(decision.reasoning.contains("WebDev Arena"));
    }

    #[tokio::test]
    async fn test_code_search_routing() {
        let selector = ModelSelector::new();

        let classification = Classification {
            category: TaskCategory::CodeSearch,
            confidence: 0.85,
            reasoning: "Code search task".to_string(),
        };

        let decision = selector
            .select(&classification, "Find all uses of")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::LocalGemma4B);
        assert_eq!(decision.estimated_cost_usd, 0.0);
    }

    #[tokio::test]
    async fn test_budget_constraint() {
        let selector = ModelSelector::new();

        let classification = Classification {
            category: TaskCategory::FrontendUI,
            confidence: 0.90,
            reasoning: "Frontend task".to_string(),
        };

        let decision = selector
            .select_with_budget_constraint(&classification, "Build a React component", 0.90)
            .await
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::LocalGemma12B);
        assert!(decision.reasoning.contains("Budget constrained"));
    }

    #[tokio::test]
    async fn test_backend_api_routing() {
        let selector = ModelSelector::new();

        let classification = Classification {
            category: TaskCategory::BackendAPI,
            confidence: 0.85,
            reasoning: "Backend API".to_string(),
        };

        let decision = selector
            .select(&classification, "Create a REST API endpoint")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::GeminiPro);
    }
}
