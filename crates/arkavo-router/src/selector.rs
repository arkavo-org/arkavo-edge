use crate::Result;
use crate::classifier::{Classification, TaskCategory};
use crate::decision::{ModelChoice, RoutingDecision};
use crate::model_discovery;

/// Provider availability status
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct ProviderAvailability {
    pub gemini: bool,
    pub anthropic: bool,
    pub deepseek: bool,
    pub kimi: bool,
}

impl ProviderAvailability {
    /// Check environment variables for API keys
    pub fn from_env() -> Self {
        Self {
            gemini: std::env::var("GEMINI_API_KEY").is_ok(),
            anthropic: std::env::var("ANTHROPIC_API_KEY").is_ok(),
            deepseek: std::env::var("DEEPSEEK_API_KEY").is_ok(),
            kimi: std::env::var("MOONSHOT_API_KEY").is_ok(),
        }
    }

    /// Check if any cloud provider is available
    pub fn has_cloud(&self) -> bool {
        self.gemini || self.anthropic || self.deepseek || self.kimi
    }
}

pub struct ModelSelector {
    pub(crate) budget_threshold: f64,
    pub(crate) availability: ProviderAvailability,
    pub(crate) gpu_available: bool,
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
            if Self::is_local_model_cached(&ModelChoice::LocalMinistral3B) {
                return ModelChoice::LocalMinistral3B;
            }
            return ModelChoice::LocalQwen3;
        }

        if prefer_larger {
            // GLM-4.7-Flash: 30B MoE, highest quality local model
            if Self::is_local_model_cached(&ModelChoice::LocalGlm47Flash)
                && Self::has_sufficient_ram_for_glm()
            {
                ModelChoice::LocalGlm47Flash
            } else if Self::is_local_model_cached(&ModelChoice::LocalQwen35_27B)
                && Self::has_sufficient_ram_for_qwen35()
            {
                ModelChoice::LocalQwen35_27B
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

    /// Fastest available local model for internal tasks (judging, synthesis, classification).
    /// These need speed, not quality — always pick the smallest cached model.
    pub fn fastest_local_model(&self) -> ModelChoice {
        if Self::is_local_model_cached(&ModelChoice::LocalQwen3) {
            ModelChoice::LocalQwen3
        } else if Self::is_local_model_cached(&ModelChoice::LocalMinistral3B) {
            ModelChoice::LocalMinistral3B
        } else {
            ModelChoice::LocalQwen3
        }
    }

    /// Check if system has sufficient RAM for Qwen3.5-27B (48GB+)
    fn has_sufficient_ram_for_qwen35() -> bool {
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            if let Ok(output) = Command::new("sysctl").arg("-n").arg("hw.memsize").output()
                && let Ok(mem_str) = String::from_utf8(output.stdout)
                && let Ok(bytes) = mem_str.trim().parse::<u64>()
            {
                return bytes >= 48 * 1024 * 1024 * 1024; // 48GB
            }
            false
        }
        #[cfg(not(target_os = "macos"))]
        {
            true
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
            ModelChoice::LocalQwen35_27B => model_discovery::is_model_cached(
                "unsloth/Qwen3.5-27B-GGUF",
                "Qwen3.5-27B-UD-Q6_K_XL.gguf",
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

    pub(crate) fn select_model_by_category(&self, classification: &Classification) -> ModelChoice {
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

            // General tasks use larger local model for better tool calling and reasoning
            TaskCategory::General => self.best_available_local_model(true),

            // Other tasks: prefer larger models when GPU available for better tool calling
            _ => self.best_available_local_model(self.gpu_available),
        }
    }

    /// All models currently feasible (cached local + API keys for cloud)
    pub fn feasible_models(&self) -> Vec<ModelChoice> {
        let mut models = Vec::new();

        // Local models
        if Self::is_local_model_cached(&ModelChoice::LocalQwen3) {
            models.push(ModelChoice::LocalQwen3);
        }
        if self.gpu_available {
            if Self::is_local_model_cached(&ModelChoice::LocalMinistral3B) {
                models.push(ModelChoice::LocalMinistral3B);
            }
            if Self::is_local_model_cached(&ModelChoice::LocalMinistral8B) {
                models.push(ModelChoice::LocalMinistral8B);
            }
            if Self::is_local_model_cached(&ModelChoice::LocalQwen35_27B)
                && Self::has_sufficient_ram_for_qwen35()
            {
                models.push(ModelChoice::LocalQwen35_27B);
            }
            if Self::is_local_model_cached(&ModelChoice::LocalGlm47Flash)
                && Self::has_sufficient_ram_for_glm()
            {
                models.push(ModelChoice::LocalGlm47Flash);
            }
        }

        // Cloud models
        if self.availability.gemini {
            models.push(ModelChoice::GeminiFlash);
            models.push(ModelChoice::GeminiPro);
        }
        if self.availability.anthropic {
            models.push(ModelChoice::ClaudeSonnet);
            models.push(ModelChoice::ClaudeOpus);
        }
        if self.availability.deepseek {
            models.push(ModelChoice::DeepSeekV32);
        }
        if self.availability.kimi {
            models.push(ModelChoice::KimiK2);
        }

        // Fallback: always include Qwen3 as baseline
        if models.is_empty() {
            models.push(ModelChoice::LocalQwen3);
        }

        models
    }
}

impl Default for ModelSelector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    fn gemini_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: true,
            anthropic: false,
            deepseek: false,
            kimi: false,
        }
    }

    fn anthropic_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: false,
            anthropic: true,
            deepseek: false,
            kimi: false,
        }
    }

    fn deepseek_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: false,
            anthropic: false,
            deepseek: true,
            kimi: false,
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
    async fn test_backend_api_routing_uses_local() {
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

    #[test]
    fn test_feasible_models_gemini_only() {
        let selector = ModelSelector::with_availability(gemini_only());
        let feasible = selector.feasible_models();
        assert!(feasible.contains(&ModelChoice::GeminiFlash));
        assert!(feasible.contains(&ModelChoice::GeminiPro));
        assert!(!feasible.contains(&ModelChoice::ClaudeSonnet));
        assert!(!feasible.contains(&ModelChoice::DeepSeekV32));
    }

    #[test]
    fn test_feasible_models_no_cloud_has_fallback() {
        let selector = ModelSelector::with_availability(ProviderAvailability::default());
        let feasible = selector.feasible_models();
        assert!(!feasible.is_empty());
    }
}
