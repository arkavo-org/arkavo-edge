use crate::Result;
use crate::classifier::{Classification, TaskCategory, TaskImportance};
use crate::decision::{ModelChoice, RoutingDecision};
use crate::model_discovery;

/// Provider availability status
#[derive(Debug, Clone, Default)]
pub struct ProviderAvailability {
    pub gemini: bool,
    pub anthropic: bool,
    pub deepseek: bool,
}

impl ProviderAvailability {
    /// Check environment variables for API keys
    pub fn from_env() -> Self {
        Self {
            gemini: std::env::var("GEMINI_API_KEY").is_ok(),
            anthropic: std::env::var("ANTHROPIC_API_KEY").is_ok(),
            deepseek: std::env::var("DEEPSEEK_API_KEY").is_ok(),
        }
    }

    /// Check if any cloud provider is available
    pub fn has_cloud(&self) -> bool {
        self.gemini || self.anthropic || self.deepseek
    }
}

pub struct ModelSelector {
    budget_threshold: f64,
    availability: ProviderAvailability,
}

impl ModelSelector {
    pub fn new() -> Self {
        Self {
            budget_threshold: 0.80,
            availability: ProviderAvailability::from_env(),
        }
    }

    pub fn with_budget_threshold(budget_threshold: f64) -> Self {
        Self {
            budget_threshold,
            availability: ProviderAvailability::from_env(),
        }
    }

    /// Create selector with explicit provider availability (for testing)
    #[cfg(test)]
    pub fn with_availability(availability: ProviderAvailability) -> Self {
        Self {
            budget_threshold: 0.80,
            availability,
        }
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

    /// Get the best available local model, checking cache availability
    fn best_available_local_model(&self, prefer_larger: bool) -> ModelChoice {
        if prefer_larger {
            // Try larger models first, fall back to smaller if not cached
            if Self::is_local_model_cached(&ModelChoice::LocalMinistral8B) {
                ModelChoice::LocalMinistral8B
            } else if Self::is_local_model_cached(&ModelChoice::LocalMinistral3B) {
                ModelChoice::LocalMinistral3B
            } else {
                ModelChoice::LocalQwen3
            }
        } else {
            ModelChoice::LocalQwen3
        }
    }

    /// Check if a local model is cached (static helper)
    fn is_local_model_cached(model: &ModelChoice) -> bool {
        match model {
            ModelChoice::LocalQwen3 => {
                model_discovery::is_model_cached("Qwen/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q8_0.gguf")
            }
            ModelChoice::LocalMinistral3B => model_discovery::is_model_cached(
                "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
                "Ministral-3-3B-Instruct-2512-Q4_K_M.gguf",
            ),
            ModelChoice::LocalMinistral8B => model_discovery::is_model_cached(
                "mistralai/Ministral-8B-Instruct-2512-GGUF",
                "Ministral-8B-Instruct-2512-Q4_K_M.gguf",
            ),
            _ => false,
        }
    }

    fn select_model_by_category(&self, classification: &Classification) -> ModelChoice {
        // Primary selection based on importance tier
        let model_by_importance = self.select_by_importance(classification.importance);

        // Category-specific overrides for specialized needs
        match classification.category {
            // Vision requires multimodal capability
            TaskCategory::VisionAnalysis => {
                if self.availability.gemini {
                    ModelChoice::GeminiFlash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    // Local models don't have vision - fall back to best available
                    model_by_importance
                }
            }
            // Code generation prefers DeepSeek when available
            TaskCategory::CodeGeneration if self.availability.deepseek => ModelChoice::DeepSeekV32,
            // Otherwise use importance-based selection
            _ => model_by_importance,
        }
    }

    /// Select model based on importance tier - uses best available at each tier
    fn select_by_importance(&self, importance: TaskImportance) -> ModelChoice {
        match importance {
            TaskImportance::Critical => {
                // Most capable model available
                if self.availability.anthropic {
                    ModelChoice::ClaudeOpus
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else if self.availability.deepseek {
                    ModelChoice::DeepSeekV32
                } else {
                    self.best_available_local_model(true)
                }
            }
            TaskImportance::High => {
                // Capable model - prefer Sonnet/Flash for speed+quality balance
                if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else if self.availability.deepseek {
                    ModelChoice::DeepSeekV32
                } else {
                    self.best_available_local_model(true)
                }
            }
            TaskImportance::Normal => {
                // Balanced - prefer fast cloud or good local
                if self.availability.gemini {
                    ModelChoice::GeminiFlash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    self.best_available_local_model(false)
                }
            }
            TaskImportance::Low => {
                // Fast and cheap - local preferred
                ModelChoice::LocalQwen3
            }
        }
    }

    fn explain_selection(&self, model: &ModelChoice, classification: &Classification) -> String {
        let importance_str = match classification.importance {
            TaskImportance::Critical => "critical",
            TaskImportance::High => "high",
            TaskImportance::Normal => "normal",
            TaskImportance::Low => "low",
        };

        let category_reason = match (classification.category, model) {
            (TaskCategory::Chat, _) => "Chat: Using most capable model for quality responses",
            (TaskCategory::FrontendUI, ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus) => {
                "Frontend task: Claude excellent for UI development"
            }
            (TaskCategory::FrontendUI, _) => "Frontend task: Gemini Flash ranks #1 on WebDev Arena",
            (TaskCategory::BackendAPI, ModelChoice::ClaudeOpus) => {
                "Backend API: Claude Opus for highest quality code"
            }
            (TaskCategory::BackendAPI, ModelChoice::ClaudeSonnet) => {
                "Backend API: Claude Sonnet for fast, high-quality code"
            }
            (TaskCategory::BackendAPI, _) => "Backend API: Selected based on importance",
            (TaskCategory::CodeSearch, _) => "Code search: Fast local model sufficient",
            (TaskCategory::SecurityScan, _) => "Security scan: Capable model for thorough analysis",
            (TaskCategory::TestGeneration, ModelChoice::ClaudeOpus) => {
                "Test generation: Claude Opus for comprehensive tests"
            }
            (TaskCategory::TestGeneration, _) => {
                "Test generation: Capable model for comprehensive tests"
            }
            (TaskCategory::Documentation, _) => "Documentation: Fast model sufficient",
            (TaskCategory::Refactoring, ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus) => {
                "Refactoring: Claude for excellent code transformations"
            }
            (TaskCategory::Refactoring, _) => "Refactoring: Selected based on importance",
            (TaskCategory::CodeGeneration, ModelChoice::DeepSeekV32) => {
                "Code generation: DeepSeek V3.2 optimized for code generation"
            }
            (TaskCategory::CodeGeneration, _) => "Code generation: Selected based on importance",
            (TaskCategory::VisionAnalysis, _) => "Vision analysis: Multimodal model required",
            (TaskCategory::General, ModelChoice::LocalQwen3) => {
                "General task: Fast local Qwen3 for quick responses"
            }
            (TaskCategory::General, _) => "General task: Selected based on importance",
        };

        let model_benefit = match model {
            ModelChoice::GeminiFlash => "Fast (3s), cost-effective ($0.003-0.006)",
            ModelChoice::GeminiPro => "Highest quality, comprehensive output ($0.009)",
            ModelChoice::ClaudeSonnet => "Fast (5s), excellent quality ($0.018-0.045)",
            ModelChoice::ClaudeOpus => "Premium quality, complex reasoning ($0.090-0.225)",
            ModelChoice::LocalQwen3 => "Ultra-fast (<1s), zero cost, TØRG-compatible",
            ModelChoice::LocalMinistral3B => "Fast (2s), zero cost, TØRG-compatible",
            ModelChoice::LocalMinistral8B => "High quality (4s), zero cost, TØRG-compatible",
            ModelChoice::LocalGemma270M => "Ultra-fast (<1s), zero cost",
            ModelChoice::LocalGemma4B => "Fast (2s), zero cost, private",
            ModelChoice::LocalGemma12B => "High quality, zero cost, private",
            ModelChoice::LocalDeepSeekCoder => {
                "Code-specialized (4s), zero cost, optimized for patches"
            }
            ModelChoice::DeepSeekV32 => "Fast (5s), cost-effective ($0.001), excellent for code",
            ModelChoice::DeepSeekV32Speciale => "Planning-optimized (5s), reasoning-only, no tools",
        };

        format!(
            "[{}] {}. Using {}: {}",
            importance_str,
            category_reason,
            model.name(),
            model_benefit
        )
    }

    /// Select the best model for a subtask based on its category (used in architect mode)
    pub fn select_for_subtask(&self, category: TaskCategory) -> ModelChoice {
        // Use the importance system for consistency
        let importance = Classification::importance_for_category(category);

        // Category-specific overrides
        match category {
            // Vision requires multimodal
            TaskCategory::VisionAnalysis => {
                if self.availability.gemini {
                    ModelChoice::GeminiFlash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    self.select_by_importance(importance)
                }
            }
            // CodeGeneration prefers DeepSeek
            TaskCategory::CodeGeneration if self.availability.deepseek => ModelChoice::DeepSeekV32,
            // Otherwise use importance-based selection
            _ => self.select_by_importance(importance),
        }
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
                    ModelChoice::LocalMinistral3B
                }
                _ => ModelChoice::LocalQwen3,
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

    fn gemini_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: true,
            anthropic: false,
            deepseek: false,
        }
    }

    fn anthropic_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: false,
            anthropic: true,
            deepseek: false,
        }
    }

    fn deepseek_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: false,
            anthropic: false,
            deepseek: true,
        }
    }

    #[tokio::test]
    async fn test_frontend_routing_gemini() {
        let selector = ModelSelector::with_availability(gemini_only());

        let classification =
            Classification::new(TaskCategory::FrontendUI, 0.90, "Frontend task".to_string());

        let decision = selector
            .select(&classification, "Build a React component")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::GeminiFlash);
        assert!(decision.reasoning.contains("WebDev Arena"));
    }

    #[tokio::test]
    async fn test_frontend_routing_anthropic() {
        let selector = ModelSelector::with_availability(anthropic_only());

        let classification =
            Classification::new(TaskCategory::FrontendUI, 0.90, "Frontend task".to_string());

        let decision = selector
            .select(&classification, "Build a React component")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::ClaudeSonnet);
        assert!(decision.reasoning.contains("Claude"));
    }

    #[tokio::test]
    async fn test_code_search_routing() {
        let selector = ModelSelector::with_availability(gemini_only());

        let classification = Classification::new(
            TaskCategory::CodeSearch,
            0.85,
            "Code search task".to_string(),
        );

        let decision = selector
            .select(&classification, "Find all uses of")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::LocalQwen3);
        assert_eq!(decision.estimated_cost_usd, 0.0);
    }

    #[tokio::test]
    async fn test_budget_constraint() {
        let selector = ModelSelector::with_availability(gemini_only());

        let classification =
            Classification::new(TaskCategory::FrontendUI, 0.90, "Frontend task".to_string());

        let decision = selector
            .select_with_budget_constraint(&classification, "Build a React component", 0.90)
            .await
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::LocalMinistral3B);
        assert!(decision.reasoning.contains("Budget constrained"));
    }

    #[tokio::test]
    async fn test_backend_api_routing_uses_gemini() {
        // BackendAPI has Normal importance - uses Gemini Flash when available
        let selector = ModelSelector::with_availability(gemini_only());

        let classification =
            Classification::new(TaskCategory::BackendAPI, 0.85, "Backend API".to_string());

        let decision = selector
            .select(&classification, "Create a REST API endpoint")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::GeminiFlash);
    }

    #[tokio::test]
    async fn test_no_cloud_falls_back_to_local() {
        let selector = ModelSelector::with_availability(ProviderAvailability::default());

        let classification =
            Classification::new(TaskCategory::FrontendUI, 0.90, "Frontend task".to_string());

        let decision = selector
            .select(&classification, "Build a React component")
            .unwrap();

        // When no cloud is available, should fall back to local
        assert!(decision.recommended_model.is_local());
    }

    #[tokio::test]
    async fn test_code_generation_routing_deepseek() {
        let selector = ModelSelector::with_availability(deepseek_only());

        let classification = Classification::new(
            TaskCategory::CodeGeneration,
            0.85,
            "Code generation task".to_string(),
        );

        let decision = selector
            .select(&classification, "Generate a function")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::DeepSeekV32);
        assert!(decision.recommended_model.is_deepseek());
    }
}
