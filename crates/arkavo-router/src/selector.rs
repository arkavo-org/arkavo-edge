use crate::Result;
use crate::classifier::{Classification, TaskCategory};
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
    gpu_available: bool,
}

impl ModelSelector {
    pub fn new() -> Self {
        Self {
            budget_threshold: 0.80,
            availability: ProviderAvailability::from_env(),
            gpu_available: Self::check_gpu_status(),
        }
    }

    pub fn with_budget_threshold(budget_threshold: f64) -> Self {
        Self {
            budget_threshold,
            availability: ProviderAvailability::from_env(),
            gpu_available: Self::check_gpu_status(),
        }
    }

    /// Create selector with explicit provider availability (for testing)
    #[cfg(test)]
    pub fn with_availability(availability: ProviderAvailability) -> Self {
        Self {
            budget_threshold: 0.80,
            availability,
            gpu_available: true, // Assume GPU available in tests
        }
    }

    /// Check GPU acceleration status via arkavo-llm
    fn check_gpu_status() -> bool {
        #[cfg(feature = "llama-cpp")]
        {
            arkavo_llm::is_gpu_accelerated()
        }
        #[cfg(not(feature = "llama-cpp"))]
        {
            false
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

    /// Get the best available local model, checking cache availability and GPU status
    ///
    /// When GPU is unavailable, skips large models (8B+) to avoid slow CPU-only inference.
    /// This prevents 20+ second waits on CPU-only devices.
    /// GLM-4.7-Flash requires 32GB+ RAM (unified memory on Apple Silicon).
    fn best_available_local_model(&self, prefer_larger: bool) -> ModelChoice {
        // If no GPU, skip large models to avoid slow CPU-only inference
        if !self.gpu_available {
            // Without GPU, prefer smaller/faster models
            if Self::is_local_model_cached(&ModelChoice::LocalMinistral3B) {
                return ModelChoice::LocalMinistral3B;
            }
            return ModelChoice::LocalQwen3;
        }

        // GPU available - use existing logic (prefer larger when requested)
        if prefer_larger {
            // Try GLM first (30B MoE, requires 32GB+ RAM), then fall back
            if Self::is_local_model_cached(&ModelChoice::LocalGlm47Flash)
                && Self::has_sufficient_ram_for_glm()
            {
                ModelChoice::LocalGlm47Flash
            } else if Self::is_local_model_cached(&ModelChoice::LocalMinistral8B) {
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

    /// Check if system has sufficient RAM for GLM-4.7-Flash (32GB+)
    fn has_sufficient_ram_for_glm() -> bool {
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            if let Ok(output) = Command::new("sysctl").arg("-n").arg("hw.memsize").output()
                && let Ok(mem_str) = String::from_utf8(output.stdout)
                && let Ok(bytes) = mem_str.trim().parse::<u64>()
            {
                return bytes >= 32 * 1024 * 1024 * 1024; // 32GB
            }
            false
        }
        #[cfg(not(target_os = "macos"))]
        {
            // On other platforms, check /proc/meminfo or assume true for now
            true
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
                "Ministral-3-3B-Instruct-2512-Q5_K_M.gguf",
            ),
            ModelChoice::LocalMinistral8B => model_discovery::is_model_cached(
                "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
                "Ministral-3-8B-Instruct-2512-Q5_K_M.gguf",
            ),
            ModelChoice::LocalGlm47Flash => model_discovery::is_model_cached(
                "unsloth/GLM-4.7-Flash-GGUF",
                "GLM-4.7-Flash-Q4_K_M.gguf",
            ),
            _ => false,
        }
    }

    /// Select the best available cloud model, preferring Anthropic > Gemini
    fn best_cloud_model(&self, prefer_pro: bool) -> ModelChoice {
        if self.availability.anthropic {
            if prefer_pro {
                ModelChoice::ClaudeOpus
            } else {
                ModelChoice::ClaudeSonnet
            }
        } else if self.availability.gemini {
            if prefer_pro {
                ModelChoice::GeminiPro
            } else {
                ModelChoice::GeminiFlash
            }
        } else {
            // No cloud available, use local (with availability check)
            self.best_available_local_model(prefer_pro)
        }
    }

    fn select_model_by_category(&self, classification: &Classification) -> ModelChoice {
        match classification.category {
            TaskCategory::FrontendUI if classification.confidence > 0.75 => {
                self.best_cloud_model(false)
            }

            // BackendAPI uses local model for simple tasks - saves cloud for complex API design
            TaskCategory::BackendAPI => ModelChoice::LocalQwen3,

            TaskCategory::CodeSearch => ModelChoice::LocalQwen3,

            // Security scan: Use smaller model without GPU to avoid slow inference
            TaskCategory::SecurityScan => {
                if self.gpu_available {
                    ModelChoice::LocalMinistral3B
                } else {
                    ModelChoice::LocalQwen3
                }
            }

            TaskCategory::CodeGeneration if self.availability.deepseek => ModelChoice::DeepSeekV32,

            TaskCategory::TestGeneration if classification.confidence > 0.70 => {
                self.best_cloud_model(true)
            }

            TaskCategory::Documentation => ModelChoice::LocalQwen3,

            TaskCategory::Refactoring if classification.confidence > 0.75 => {
                self.best_cloud_model(false)
            }

            // Code generation fallback: Use smaller model without GPU
            TaskCategory::CodeGeneration => {
                if self.gpu_available {
                    ModelChoice::LocalMinistral3B
                } else {
                    ModelChoice::LocalQwen3
                }
            }

            TaskCategory::VisionAnalysis => self.best_cloud_model(false),

            // General tasks with high confidence use larger local model for better reasoning
            TaskCategory::General if classification.confidence > 0.7 => {
                self.best_available_local_model(true)
            }

            // Low confidence general tasks use fast model
            _ => ModelChoice::LocalQwen3,
        }
    }

    fn explain_selection(&self, model: &ModelChoice, classification: &Classification) -> String {
        let category_reason = match (classification.category, model) {
            (TaskCategory::FrontendUI, ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus) => {
                "Frontend task: Claude Sonnet excellent for UI development"
            }
            (TaskCategory::FrontendUI, _) => "Frontend task: Gemini Flash ranks #1 on WebDev Arena",
            (TaskCategory::BackendAPI, ModelChoice::ClaudeOpus) => {
                "Backend API: Claude Opus for highest quality code"
            }
            (TaskCategory::BackendAPI, ModelChoice::ClaudeSonnet) => {
                "Backend API: Claude Sonnet for fast, high-quality code"
            }
            (TaskCategory::BackendAPI, _) => "Backend API: Gemini Pro provides highest quality",
            (TaskCategory::CodeSearch, _) => "Code search: Local Gemma 4B is fast and free",
            (TaskCategory::SecurityScan, _) => "Security scan: Local Gemma 4B for privacy",
            (TaskCategory::TestGeneration, ModelChoice::ClaudeOpus) => {
                "Test generation: Claude Opus for comprehensive tests"
            }
            (TaskCategory::TestGeneration, _) => {
                "Test generation: Gemini Pro for comprehensive tests"
            }
            (TaskCategory::Documentation, _) => "Documentation: Local Gemma 4B sufficient",
            (TaskCategory::Refactoring, ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus) => {
                "Refactoring: Claude for excellent code transformations"
            }
            (TaskCategory::Refactoring, _) => "Refactoring: Gemini Flash for quick iterations",
            (TaskCategory::CodeGeneration, ModelChoice::DeepSeekV32) => {
                "Code generation: DeepSeek V3.2 with tool support for code generation"
            }
            (TaskCategory::CodeGeneration, _) => {
                "Code generation: DeepSeek-Coder V2 Lite optimized for code/patch generation"
            }
            (TaskCategory::VisionAnalysis, _) => {
                "Vision analysis: Gemini Flash with multimodal support"
            }
            (TaskCategory::General, ModelChoice::LocalQwen3) => {
                "General task: Fast local Qwen3 for quick responses"
            }
            (TaskCategory::General, _) => "General task: Local model for efficiency",
        };

        let model_benefit = match model {
            ModelChoice::GeminiFlash => "Fast (3s), cost-effective ($0.003-0.006)",
            ModelChoice::GeminiPro => "Highest quality, comprehensive output ($0.009)",
            ModelChoice::ClaudeSonnet => "Fast (5s), excellent quality ($0.018-0.045)",
            ModelChoice::ClaudeOpus => "Premium quality, complex reasoning ($0.090-0.225)",
            ModelChoice::LocalQwen3 => "Ultra-fast (<1s), zero cost, TØRG-compatible",
            ModelChoice::LocalMinistral3B => "Fast (2s), zero cost, TØRG-compatible",
            ModelChoice::LocalMinistral8B => "High quality (4s), zero cost, TØRG-compatible",
            ModelChoice::LocalGlm47Flash => "30B MoE reasoning (8s), zero cost, excellent quality",
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
            "{}. Using {}: {}",
            category_reason,
            model.name(),
            model_benefit
        )
    }

    /// Select the best model for a subtask based on its category (used in architect mode)
    pub fn select_for_subtask(&self, category: TaskCategory) -> ModelChoice {
        match category {
            // Frontend tasks: Use cheaper, fast models
            TaskCategory::FrontendUI => {
                if self.availability.gemini {
                    ModelChoice::GeminiFlash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // Backend/Security/Tests: Use more capable models
            TaskCategory::BackendAPI
            | TaskCategory::SecurityScan
            | TaskCategory::TestGeneration => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeOpus
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else {
                    ModelChoice::LocalMinistral8B
                }
            }

            // Documentation: Use cheaper models
            TaskCategory::Documentation => {
                if self.availability.gemini {
                    ModelChoice::GeminiFlash
                } else {
                    ModelChoice::LocalQwen3
                }
            }

            // Refactoring: Use balanced models
            TaskCategory::Refactoring => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // CodeGeneration: DeepSeek V3.2 excels at code generation with tools
            TaskCategory::CodeGeneration => {
                if self.availability.deepseek {
                    ModelChoice::DeepSeekV32
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else {
                    ModelChoice::LocalDeepSeekCoder
                }
            }

            // Code search: Local model is sufficient
            TaskCategory::CodeSearch => ModelChoice::LocalQwen3,

            // Vision: Needs multimodal
            TaskCategory::VisionAnalysis => {
                if self.availability.gemini {
                    ModelChoice::GeminiFlash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // General: Use fast local model
            TaskCategory::General => ModelChoice::LocalQwen3,

            // Fallback for any future categories - prefer local
            #[allow(unreachable_patterns)]
            _ => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::GeminiFlash
                } else {
                    ModelChoice::LocalQwen3
                }
            }
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
    async fn test_backend_api_routing_uses_local() {
        // BackendAPI now uses local models for simple tasks (saves cloud for complex API design)
        let selector = ModelSelector::with_availability(gemini_only());

        let classification =
            Classification::new(TaskCategory::BackendAPI, 0.85, "Backend API".to_string());

        let decision = selector
            .select(&classification, "Create a REST API endpoint")
            .unwrap();

        assert_eq!(decision.recommended_model, ModelChoice::LocalQwen3);
        assert_eq!(decision.estimated_cost_usd, 0.0);
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
