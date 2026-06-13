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
    /// Per-agent memory budget in bytes. Models exceeding this are excluded from
    /// feasible set. 0 means unconstrained (backward compat).
    pub(crate) max_memory_bytes: std::sync::atomic::AtomicU64,
}

impl ModelSelector {
    pub fn new() -> Self {
        Self {
            budget_threshold: 0.80,
            availability: ProviderAvailability::from_env(),
            gpu_available: Self::check_gpu_status(),
            max_memory_bytes: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn with_budget_threshold(budget_threshold: f64) -> Self {
        Self {
            budget_threshold,
            availability: ProviderAvailability::from_env(),
            gpu_available: Self::check_gpu_status(),
            max_memory_bytes: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Update the per-agent memory budget. Models whose weight files exceed
    /// this limit are excluded from the feasible set.
    pub fn set_memory_budget(&self, bytes: u64) {
        self.max_memory_bytes
            .store(bytes, std::sync::atomic::Ordering::Relaxed);
    }

    /// Create selector with explicit provider availability (for testing)
    #[cfg(test)]
    pub fn with_availability(availability: ProviderAvailability) -> Self {
        Self {
            budget_threshold: 0.80,
            availability,
            gpu_available: true, // Assume GPU available in tests
            max_memory_bytes: std::sync::atomic::AtomicU64::new(0),
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
        let mem_budget = self
            .max_memory_bytes
            .load(std::sync::atomic::Ordering::Relaxed);

        let fits_budget = |m: &ModelChoice| mem_budget == 0 || m.size_bytes() <= mem_budget;

        // If no GPU, skip large models to avoid slow CPU-only inference
        if !self.gpu_available {
            if Self::is_local_model_cached(&ModelChoice::LocalMinistral3B)
                && fits_budget(&ModelChoice::LocalMinistral3B)
            {
                return ModelChoice::LocalMinistral3B;
            }
            return ModelChoice::LocalQwen3;
        }

        if prefer_larger {
            // GLM-4.7-Flash: 30B MoE, highest quality local model
            if Self::is_local_model_cached(&ModelChoice::LocalGlm47Flash)
                && Self::has_sufficient_ram(32)
                && fits_budget(&ModelChoice::LocalGlm47Flash)
            {
                ModelChoice::LocalGlm47Flash
            } else if Self::is_local_model_cached(&ModelChoice::LocalQwen35_27B)
                && Self::has_sufficient_ram(48)
                && fits_budget(&ModelChoice::LocalQwen35_27B)
            {
                ModelChoice::LocalQwen35_27B
            } else if Self::is_local_model_cached(&ModelChoice::LocalMinistral8B)
                && fits_budget(&ModelChoice::LocalMinistral8B)
            {
                ModelChoice::LocalMinistral8B
            } else if Self::is_local_model_cached(&ModelChoice::LocalMinistral3B)
                && fits_budget(&ModelChoice::LocalMinistral3B)
            {
                ModelChoice::LocalMinistral3B
            } else {
                ModelChoice::LocalQwen3
            }
        } else {
            ModelChoice::LocalQwen3
        }
    }

    /// Preference order for the fast internal model (judging, synthesis, classification),
    /// most-preferred first. Gemma 4 E2B leads because first-run setup provisions it as the
    /// "Small (fast routing)" model; the legacy entries keep older installs working without a
    /// download. The fallback when none are cached MUST stay a setup-provisioned model.
    const FAST_LOCAL_PREFERENCE: [ModelChoice; 3] = [
        ModelChoice::LocalGemma4E2B,
        ModelChoice::LocalMinistral3B,
        ModelChoice::LocalQwen3,
    ];

    /// Fastest available local model for internal tasks (judging, synthesis, classification).
    /// Prefers a cached model from [`Self::FAST_LOCAL_PREFERENCE`]; falls back to Gemma 4 E2B —
    /// the model first-run setup downloads — so chat never silently pulls an un-provisioned
    /// model the user never opted into.
    pub fn fastest_local_model(&self) -> ModelChoice {
        Self::pick_fast_local_model(Self::is_local_model_cached)
    }

    /// Policy half of [`Self::fastest_local_model`], split out so the preference order and
    /// fallback can be unit-tested without touching the on-disk model cache.
    fn pick_fast_local_model(is_cached: impl Fn(&ModelChoice) -> bool) -> ModelChoice {
        Self::FAST_LOCAL_PREFERENCE
            .into_iter()
            .find(|m| is_cached(m))
            .unwrap_or(ModelChoice::LocalGemma4E2B)
    }

    /// Check if system has at least `min_gb` of RAM
    fn has_sufficient_ram(min_gb: u64) -> bool {
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            if let Ok(output) = Command::new("sysctl").arg("-n").arg("hw.memsize").output()
                && let Ok(mem_str) = String::from_utf8(output.stdout)
                && let Ok(bytes) = mem_str.trim().parse::<u64>()
            {
                return bytes >= min_gb * 1024 * 1024 * 1024;
            }
            false
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = min_gb;
            true
        }
    }

    /// Check if a local model is cached (static helper)
    fn is_local_model_cached(model: &ModelChoice) -> bool {
        match (model.repo_id(), model.gguf_filename()) {
            (Some(repo), Some(file)) => model_discovery::is_model_cached(repo, file),
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
                ModelChoice::Gemini35Flash
            }
        } else {
            // No cloud available, use local (with availability check)
            self.best_available_local_model(prefer_pro)
        }
    }

    pub(crate) fn select_model_by_category(&self, classification: &Classification) -> ModelChoice {
        let mem_budget = self
            .max_memory_bytes
            .load(std::sync::atomic::Ordering::Relaxed);

        let model = match classification.category {
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

            // Game/simulation and general tasks use larger local model
            TaskCategory::GameSimulation | TaskCategory::General => {
                self.best_available_local_model(true)
            }

            // Other tasks: prefer larger models when GPU available for better tool calling
            _ => self.best_available_local_model(self.gpu_available),
        };

        // Enforce per-agent memory budget on the selected model
        model.downgrade_for_budget(mem_budget)
    }

    /// All models currently feasible (cached local + API keys for cloud).
    ///
    /// When `max_memory_bytes` is set (> 0), local models whose weight files
    /// exceed the budget are excluded so Thompson Sampling never selects a
    /// model that would blow the agent's memory allocation.
    pub fn feasible_models(&self) -> Vec<ModelChoice> {
        let mut models = Vec::new();
        let mem_budget = self
            .max_memory_bytes
            .load(std::sync::atomic::Ordering::Relaxed);

        // Local models (smallest first)
        if Self::is_local_model_cached(&ModelChoice::LocalQwen3) {
            models.push(ModelChoice::LocalQwen3);
        }
        if Self::is_local_model_cached(&ModelChoice::LocalGemma4E2B) {
            models.push(ModelChoice::LocalGemma4E2B);
        }
        if self.gpu_available {
            if Self::is_local_model_cached(&ModelChoice::LocalMinistral3B) {
                models.push(ModelChoice::LocalMinistral3B);
            }
            // Gemma-4-E4B excluded: 1/8 tool accuracy (benchmark), needs grammar-constrained
            // generation. Re-enable when PEG output parser lands.
            // if Self::is_local_model_cached(&ModelChoice::LocalGemma4E4B) {
            //     models.push(ModelChoice::LocalGemma4E4B);
            // }
            if Self::is_local_model_cached(&ModelChoice::LocalMinistral8B) {
                models.push(ModelChoice::LocalMinistral8B);
            }
            if Self::is_local_model_cached(&ModelChoice::LocalGemma4_26B) {
                models.push(ModelChoice::LocalGemma4_26B);
            }
            if Self::is_local_model_cached(&ModelChoice::LocalGemma4_31B) {
                models.push(ModelChoice::LocalGemma4_31B);
            }
            if Self::is_local_model_cached(&ModelChoice::LocalGemma4_12B) {
                models.push(ModelChoice::LocalGemma4_12B);
            }
            if Self::is_local_model_cached(&ModelChoice::LocalQwen35_27B)
                && Self::has_sufficient_ram(48)
            {
                models.push(ModelChoice::LocalQwen35_27B);
            }
            if Self::is_local_model_cached(&ModelChoice::LocalGlm47Flash)
                && Self::has_sufficient_ram(32)
            {
                models.push(ModelChoice::LocalGlm47Flash);
            }
        }

        // Per-agent memory budget: exclude local models that exceed the allocation
        if mem_budget > 0 {
            let before = models.len();
            models.retain(|m| m.size_bytes() == 0 || m.size_bytes() <= mem_budget);
            if models.len() < before {
                tracing::info!(
                    budget_mb = mem_budget / (1024 * 1024),
                    kept = models.len(),
                    excluded = before - models.len(),
                    "Memory budget: excluded models exceeding per-agent allocation"
                );
            }
        }

        // Cloud models (unconstrained by memory)
        if self.availability.gemini {
            // Gemini 3.5 Flash (May 2026) ships as four distinct Thompson
            // Sampling arms — one per thinking tier — so the learning
            // module can converge on the right cost/quality point per
            // task category. `Gemini35Flash` (low tier) is the production
            // default; the others are opt-in via learning.
            models.push(ModelChoice::Gemini35Flash);
            models.push(ModelChoice::Gemini35FlashMinimal);
            models.push(ModelChoice::Gemini35FlashMedium);
            models.push(ModelChoice::Gemini35FlashHigh);
            // Legacy Flash alias kept around for cost-tier fallback.
            models.push(ModelChoice::GeminiFlash);
        }
        if self.availability.anthropic {
            models.push(ModelChoice::ClaudeSonnet);
            models.push(ModelChoice::ClaudeOpus);
            // Fable 5 is 2x Opus pricing; exposed as a Thompson Sampling arm
            // so the learning module can converge on the task categories
            // where the capability gain justifies the premium. It is never a
            // category default — it's reached via learning, escalation, or an
            // explicit AGENTS.md `model:` hint.
            models.push(ModelChoice::ClaudeFable5);
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
    use arkavo_test_macros::spec;

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

    #[spec("ROUTER-001")]
    #[tokio::test]
    async fn test_frontend_routing_gemini() {
        let selector = ModelSelector::with_availability(gemini_only());
        let classification =
            Classification::new(TaskCategory::FrontendUI, 0.90, "Frontend task".to_string());
        let decision = selector
            .select(&classification, "Build a React component")
            .unwrap();
        assert_eq!(decision.recommended_model, ModelChoice::Gemini35Flash);
        assert!(decision.reasoning.contains("WebDev Arena"));
    }

    #[spec("ROUTER-001")]
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

    #[spec("ROUTER-001")]
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

    #[spec("ROUTER-001")]
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

    #[spec("ROUTER-001")]
    #[spec("ROUTER-003")]
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

    #[spec("ROUTER-001")]
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
        assert!(feasible.contains(&ModelChoice::Gemini35Flash));
        assert!(feasible.contains(&ModelChoice::GeminiFlash));
        // GeminiPro removed from feasible set (Flash only) in d4227709
        assert!(!feasible.contains(&ModelChoice::GeminiPro));
        assert!(!feasible.contains(&ModelChoice::ClaudeSonnet));
        assert!(!feasible.contains(&ModelChoice::DeepSeekV32));
    }

    #[spec("ROUTER-003")]
    #[test]
    fn test_feasible_models_no_cloud_has_fallback() {
        let selector = ModelSelector::with_availability(ProviderAvailability::default());
        let feasible = selector.feasible_models();
        assert!(!feasible.is_empty());
    }

    // Regression: a fresh install provisions Gemma 4 E2B + Gemma 4 12B (no Ministral/Qwen).
    // The fast-model selector must not fall through to a hardcoded Ministral 3B, which made
    // `arkavo chat` silently download an un-provisioned model on first use.
    #[spec("ROUTER-003")]
    #[test]
    fn test_fast_local_model_falls_back_to_provisioned_gemma() {
        let nothing_cached = |_: &ModelChoice| false;
        assert_eq!(
            ModelSelector::pick_fast_local_model(nothing_cached),
            ModelChoice::LocalGemma4E2B,
        );
    }

    #[spec("ROUTER-003")]
    #[test]
    fn test_fast_local_model_uses_cached_gemma_e2b() {
        // Fresh install: only the two setup models are present.
        let gemma_cached = |m: &ModelChoice| {
            matches!(
                m,
                ModelChoice::LocalGemma4E2B | ModelChoice::LocalGemma4_12B
            )
        };
        assert_eq!(
            ModelSelector::pick_fast_local_model(gemma_cached),
            ModelChoice::LocalGemma4E2B,
        );
    }

    #[spec("ROUTER-003")]
    #[test]
    fn test_fast_local_model_honors_legacy_ministral_install() {
        // Older install with only Ministral 3B cached still resolves to it (no download).
        let ministral_cached = |m: &ModelChoice| matches!(m, ModelChoice::LocalMinistral3B);
        assert_eq!(
            ModelSelector::pick_fast_local_model(ministral_cached),
            ModelChoice::LocalMinistral3B,
        );
    }
}
