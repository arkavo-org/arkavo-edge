use crate::Result;
use crate::classifier::{Classification, TaskCategory};
use crate::decision::{ModelChoice, RoutingDecision};
pub use crate::selector_local::LocalWeights;

/// Provider availability status
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct ProviderAvailability {
    pub gemini: bool,
    pub anthropic: bool,
    pub deepseek: bool,
    pub kimi: bool,
    pub glm: bool,
    pub xai: bool,
    pub openai: bool,
}

impl ProviderAvailability {
    /// Check environment variables for API keys
    pub fn from_env() -> Self {
        Self {
            gemini: cfg!(feature = "gemini") && std::env::var("GEMINI_API_KEY").is_ok(),
            anthropic: cfg!(feature = "llm-remote") && std::env::var("ANTHROPIC_API_KEY").is_ok(),
            deepseek: cfg!(feature = "deepseek") && std::env::var("DEEPSEEK_API_KEY").is_ok(),
            kimi: cfg!(feature = "kimi") && std::env::var("MOONSHOT_API_KEY").is_ok(),
            // Gated on the `glm` feature so the arm isn't marked feasible in a
            // build that can't instantiate it (instantiation is cfg(glm)).
            glm: cfg!(feature = "glm") && std::env::var("GLM_API_KEY").is_ok(),
            // Same pattern for xAI Grok: feature + key.
            openai: cfg!(feature = "openai")
                && std::env::var("OPENAI_API_KEY").is_ok_and(|key| !key.trim().is_empty()),
            xai: cfg!(feature = "xai") && std::env::var("XAI_API_KEY").is_ok(),
        }
    }

    /// Check if any cloud provider is available
    pub fn has_cloud(&self) -> bool {
        self.gemini
            || self.anthropic
            || self.deepseek
            || self.kimi
            || self.glm
            || self.xai
            || self.openai
    }
}

pub struct ModelSelector {
    pub(crate) budget_threshold: f64,
    pub(crate) availability: ProviderAvailability,
    pub(crate) gpu_available: bool,
    /// Per-agent memory budget in bytes. Models exceeding this are excluded from
    /// feasible set. 0 means unconstrained (backward compat).
    pub(crate) max_memory_bytes: std::sync::atomic::AtomicU64,
    pub(crate) local_weights: LocalWeights,
}

impl ModelSelector {
    pub fn new() -> Self {
        Self::with_budget_threshold(0.80)
    }

    pub fn with_budget_threshold(budget_threshold: f64) -> Self {
        Self {
            budget_threshold,
            availability: ProviderAvailability::from_env(),
            gpu_available: Self::check_gpu_status(),
            max_memory_bytes: std::sync::atomic::AtomicU64::new(0),
            local_weights: LocalWeights::HuggingFaceCache,
        }
    }

    /// Update the per-agent memory budget. Models whose weight files exceed
    /// this limit are excluded from the feasible set.
    pub fn set_memory_budget(&self, bytes: u64) {
        self.max_memory_bytes
            .store(bytes, std::sync::atomic::Ordering::Relaxed);
    }

    /// Selection seam: build a selector from an explicit provider availability
    /// and an explicit answer to "are local weights cached", so a caller can
    /// assert routing behaviour without depending on the machine's API-key
    /// environment or HuggingFace cache. Install it with
    /// [`crate::Router::with_selector`].
    pub fn with_availability(availability: ProviderAvailability, local_cached: bool) -> Self {
        Self::with_parts(availability, LocalWeights::Fixed(local_cached))
    }

    /// Test-oriented seam: build a selector from an explicit provider
    /// availability and an explicit local-weights answer source, so a test
    /// can assert routing behaviour deterministically. Only `availability`
    /// and `local_weights` are caller-controlled; `budget_threshold` is
    /// fixed at `0.80`, `gpu_available` is fixed `true` (tests should not
    /// depend on the runner's hardware), and `max_memory_bytes` is
    /// unconstrained (`0`). A caller that needs to preserve an existing
    /// selector's real hardware/memory state — e.g. propagating an
    /// orchestrator's own selector to a router it builds internally — must
    /// use [`ModelSelector::snapshot`] instead, not this constructor.
    pub fn with_parts(availability: ProviderAvailability, local_weights: LocalWeights) -> Self {
        Self {
            budget_threshold: 0.80,
            availability,
            gpu_available: true,
            max_memory_bytes: std::sync::atomic::AtomicU64::new(0),
            local_weights,
        }
    }

    /// Full copy of this selector, including its runtime-mutable memory
    /// budget — every field, not just availability and local-weights. For
    /// propagating an existing selector (e.g. an orchestrator's) to a
    /// `Router` built internally, where reconstructing via [`Self::with_parts`]
    /// would silently drop real GPU/memory state and reintroduce host-only
    /// defaults.
    pub fn snapshot(&self) -> Self {
        Self {
            budget_threshold: self.budget_threshold,
            availability: self.availability.clone(),
            gpu_available: self.gpu_available,
            max_memory_bytes: std::sync::atomic::AtomicU64::new(
                self.max_memory_bytes
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
            local_weights: self.local_weights,
        }
    }

    /// How this selector answers local-weight cache questions.
    pub fn local_weights(&self) -> LocalWeights {
        self.local_weights
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

    /// Select the best available cloud model, preferring Anthropic > Gemini.
    ///
    /// Falls back to the best arm this device can run locally when no cloud
    /// provider is configured.
    fn best_cloud_model(&self, prefer_pro: bool) -> ModelChoice {
        best_configured_cloud_model(&self.availability, prefer_pro)
            .unwrap_or_else(|| self.best_available_local_model(prefer_pro))
    }

    /// The cloud arm that augments local inference, or `None` when no cloud
    /// provider is configured.
    pub(crate) fn cloud_augmentation_model(&self) -> Option<ModelChoice> {
        cloud_augmentation_model(&self.availability)
    }

    /// Cloud arms this selector's credentials make feasible. Unconstrained by
    /// the memory budget: nothing runs on this device.
    pub(crate) fn feasible_cloud_models(&self) -> Vec<ModelChoice> {
        let mut models = Vec::new();
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
        if self.availability.glm {
            // GLM-5.2 enters as a single Thompson Sampling arm (cold-start
            // cap). The learning module decides where its quality/cost point
            // beats the other low-cost cloud arms (DeepSeek, Gemini Flash).
            models.push(ModelChoice::Glm52);
        }
        if self.availability.openai {
            models.push(ModelChoice::Gpt6Astra);
        }
        if self.availability.xai {
            // Grok 4.7 base arm (low effort) plus the xhigh companion so
            // Thompson Sampling can learn when max-depth reasoning pays off.
            models.push(ModelChoice::Grok47);
            models.push(ModelChoice::Grok47Xhigh);
        }
        models
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

        let model = model.downgrade_for_budget(mem_budget);
        if model.is_local() && !self.is_local_model_cached(&model) {
            // Reuse provisioned weights, or the first-run local default. A cloud
            // key must not turn a missing preferred local model into cloud spend.
            self.fastest_local_model().downgrade_for_budget(mem_budget)
        } else {
            model
        }
    }
}

/// Best cloud arm for the configured providers, or `None` when none is
/// configured.
///
/// A free function of availability alone: a caller that holds its own
/// provider set — the architect planner, which can be configured explicitly —
/// asks it directly instead of constructing a throwaway selector.
pub(crate) fn best_configured_cloud_model(
    availability: &ProviderAvailability,
    prefer_pro: bool,
) -> Option<ModelChoice> {
    if availability.anthropic {
        Some(if prefer_pro {
            ModelChoice::ClaudeOpus
        } else {
            ModelChoice::ClaudeSonnet
        })
    } else if availability.gemini {
        Some(if prefer_pro {
            ModelChoice::GeminiPro
        } else {
            ModelChoice::Gemini35Flash
        })
    } else if availability.deepseek {
        Some(ModelChoice::DeepSeekV32)
    } else if availability.kimi {
        Some(ModelChoice::KimiK2)
    } else if availability.glm {
        Some(ModelChoice::Glm52)
    } else if availability.xai {
        Some(ModelChoice::Grok47)
    } else if availability.openai {
        Some(ModelChoice::Gpt6Astra)
    } else {
        None
    }
}

/// The cloud arm that augments local inference for these providers.
pub(crate) fn cloud_augmentation_model(availability: &ProviderAvailability) -> Option<ModelChoice> {
    best_configured_cloud_model(availability, false)
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
            glm: false,
            xai: false,
            openai: false,
        }
    }

    fn anthropic_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: false,
            anthropic: true,
            deepseek: false,
            kimi: false,
            glm: false,
            xai: false,
            openai: false,
        }
    }

    fn deepseek_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: false,
            anthropic: false,
            deepseek: true,
            kimi: false,
            glm: false,
            xai: false,
            openai: false,
        }
    }

    fn glm_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: false,
            anthropic: false,
            deepseek: false,
            kimi: false,
            glm: true,
            xai: false,
            openai: false,
        }
    }

    fn xai_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: false,
            anthropic: false,
            deepseek: false,
            kimi: false,
            glm: false,
            xai: true,
            openai: false,
        }
    }

    #[test]
    fn cloud_credentials_do_not_replace_local_defaults() {
        for cached in [false, true] {
            let selector = ModelSelector::with_availability(
                ProviderAvailability {
                    openai: true,
                    ..Default::default()
                },
                cached,
            );
            assert!(selector.default_execution_model().is_local());
            assert_eq!(
                selector.cloud_augmentation_model(),
                Some(ModelChoice::Gpt6Astra)
            );
            for category in [
                TaskCategory::CodeSearch,
                TaskCategory::BackendAPI,
                TaskCategory::General,
            ] {
                let classification = Classification::new(category, 0.9, "task".into());
                assert!(
                    selector
                        .select_model_by_category(&classification)
                        .is_local()
                );
            }
        }
    }

    #[test]
    fn astra_does_not_replace_existing_cloud_preference() {
        let selector = ModelSelector::with_availability(
            ProviderAvailability {
                openai: true,
                gemini: true,
                ..ProviderAvailability::default()
            },
            false,
        );
        assert_eq!(selector.best_cloud_model(false), ModelChoice::Gemini35Flash);
    }

    #[spec("ROUTER-001")]
    #[tokio::test]
    async fn test_frontend_routing_gemini() {
        let selector = ModelSelector::with_availability(gemini_only(), false);
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
        let selector = ModelSelector::with_availability(anthropic_only(), false);
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
        let selector = ModelSelector::with_availability(gemini_only(), false);
        let classification = Classification::new(
            TaskCategory::CodeSearch,
            0.85,
            "Code search task".to_string(),
        );
        let decision = selector
            .select(&classification, "Find all uses of")
            .unwrap();
        assert_eq!(decision.recommended_model, ModelChoice::LocalGemma4E2B);
        assert_eq!(decision.estimated_cost_usd, 0.0);
    }

    #[spec("ROUTER-001")]
    #[tokio::test]
    async fn test_backend_api_routing_uses_local() {
        let selector = ModelSelector::with_availability(gemini_only(), false);
        let classification =
            Classification::new(TaskCategory::BackendAPI, 0.85, "Backend API".to_string());
        let decision = selector
            .select(&classification, "Create a REST API endpoint")
            .unwrap();
        assert_eq!(decision.recommended_model, ModelChoice::LocalGemma4E2B);
        assert_eq!(decision.estimated_cost_usd, 0.0);
    }

    #[spec("ROUTER-001")]
    #[spec("ROUTER-003")]
    #[tokio::test]
    async fn test_no_cloud_falls_back_to_local() {
        let selector = ModelSelector::with_availability(ProviderAvailability::default(), false);
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
        let selector = ModelSelector::with_availability(deepseek_only(), false);
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
        let selector = ModelSelector::with_availability(gemini_only(), false);
        let feasible = selector.feasible_models();
        assert!(feasible.contains(&ModelChoice::Gemini35Flash));
        assert!(feasible.contains(&ModelChoice::GeminiFlash));
        // GeminiPro removed from feasible set (Flash only) in d4227709
        assert!(!feasible.contains(&ModelChoice::GeminiPro));
        assert!(!feasible.contains(&ModelChoice::ClaudeSonnet));
        assert!(!feasible.contains(&ModelChoice::DeepSeekV32));
    }

    #[test]
    fn test_feasible_models_glm_only() {
        let selector = ModelSelector::with_availability(glm_only(), false);
        let feasible = selector.feasible_models();
        assert!(feasible.contains(&ModelChoice::Glm52));
        assert!(!feasible.contains(&ModelChoice::DeepSeekV32));
        assert!(!feasible.contains(&ModelChoice::ClaudeSonnet));
    }

    #[test]
    fn test_feasible_models_xai_only() {
        let selector = ModelSelector::with_availability(xai_only(), false);
        let feasible = selector.feasible_models();
        assert!(feasible.contains(&ModelChoice::Grok47));
        assert!(feasible.contains(&ModelChoice::Grok47Xhigh));
        assert!(!feasible.contains(&ModelChoice::Glm52));
        assert!(!feasible.contains(&ModelChoice::ClaudeSonnet));
    }

    #[test]
    fn test_provider_availability_glm_counts_as_cloud() {
        assert!(glm_only().has_cloud());
    }

    #[test]
    fn test_provider_availability_xai_counts_as_cloud() {
        assert!(xai_only().has_cloud());
    }

    /// Regression: the feasible set used to end with an unconditional
    /// `LocalQwen3` whose weights nobody had checked, so a device holding
    /// nothing handed Thompson Sampling an arm that could only be served by
    /// downloading it. A bare device now has nothing feasible, and the caller
    /// turns that into a refusal.
    #[spec("ROUTER-003")]
    #[test]
    fn test_feasible_models_no_cloud_no_weights_is_empty() {
        let selector = ModelSelector::with_availability(ProviderAvailability::default(), false);
        assert!(selector.feasible_models().is_empty());
    }

    /// A device provisioned outside the local shortlist is still feasible on
    /// what it actually holds.
    #[spec("ROUTER-003")]
    #[test]
    fn test_feasible_models_names_an_arm_outside_the_shortlist() {
        let selector = ModelSelector::with_parts(
            ProviderAvailability::default(),
            LocalWeights::Only(&[ModelChoice::LocalGemma4E4B]),
        );
        assert_eq!(
            selector.feasible_models(),
            vec![ModelChoice::LocalGemma4E4B]
        );
    }
}
