use crate::Result;
use crate::classifier::{Classification, TaskCategory};
use crate::decision::{ModelChoice, RoutingDecision};
use crate::learning::LearningModule;
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

            // General tasks use larger local model for better tool calling and reasoning
            TaskCategory::General => self.best_available_local_model(true),

            // Other tasks: prefer larger models when GPU available for better tool calling
            _ => self.best_available_local_model(self.gpu_available),
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
            (TaskCategory::CodeReview, _) => "Code review: Capable model for thorough analysis",
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
            ModelChoice::KimiK2 => "Fast (5s), 256K context, thinking mode support",
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

    /// Select model using Thompson Sampling from the learning module.
    ///
    /// Budget and exclusions are applied to the feasible set *before* sampling,
    /// so Thompson Sampling never picks a model that will be rejected downstream.
    ///
    /// `excluded` contains model names that are temporarily unavailable (cooldown
    /// after availability failures like 429s or timeouts). These are NOT quality
    /// signals — they don't update Beta priors.
    pub async fn select_adaptive(
        &self,
        learning: &LearningModule,
        classification: &Classification,
        budget_usage: f64,
        excluded: &[String],
    ) -> Result<RoutingDecision> {
        let mut feasible = self.feasible_models();

        // Budget gate: over threshold → local only
        if budget_usage > self.budget_threshold {
            feasible.retain(|m| m.is_local());
        }

        // Availability gate: exclude cooled-down models
        if !excluded.is_empty() {
            feasible.retain(|m| !excluded.iter().any(|e| e == m.name()));
        }

        // Ensure at least one model
        if feasible.is_empty() {
            feasible.push(ModelChoice::LocalQwen3);
        }

        if feasible.len() == 1 {
            let model = feasible
                .into_iter()
                .next()
                .unwrap_or(ModelChoice::LocalQwen3);
            let reasoning = format!("Single feasible model: {}", model.name());
            return Ok(RoutingDecision::new(
                model,
                classification.category,
                classification.confidence,
                reasoning,
            ));
        }

        let model_ids: Vec<String> = feasible.iter().map(|m| m.name().to_string()).collect();
        let category = Some(classification.category.as_str());

        tracing::info!(
            feasible_count = feasible.len(),
            category = classification.category.as_str(),
            models = %model_ids.join(", "),
            "Thompson Sampling: evaluating feasible models"
        );

        let ranked = learning.rank_agents(&model_ids, category).await;

        // Log full ranking for diagnostics
        for (i, (name, score)) in ranked.iter().enumerate() {
            tracing::info!(
                rank = i + 1,
                model = %name,
                score = format!("{score:.4}").as_str(),
                category = classification.category.as_str(),
                "Thompson Sampling rank"
            );
        }

        if let Some((best_model_name, score)) = ranked.first() {
            let model = feasible
                .iter()
                .find(|m| m.name() == best_model_name.as_str())
                .cloned()
                .unwrap_or_else(|| self.select_model_by_category(classification));

            let reasoning = format!(
                "Thompson Sampling: {} (score {:.3}, category {})",
                model.name(),
                score,
                classification.category.as_str()
            );
            Ok(RoutingDecision::new(
                model,
                classification.category,
                classification.confidence,
                reasoning,
            ))
        } else {
            self.select(classification, "")
        }
    }
}

impl Default for ModelSelector {
    fn default() -> Self {
        Self::new()
    }
}

/// Heuristic quality score for a model response (0.0 to 1.0, no LLM call).
///
/// Category-aware: complex task types (test generation, code review, security
/// scan) expect longer, more detailed responses. A short reply that's fine for
/// a simple search query is inadequate for a comprehensive security audit.
/// This prevents fallback models from getting inflated priors just because
/// they returned *something*.
pub fn compute_response_quality(response: &str, latency_ms: u64, category: &str) -> f64 {
    if response.trim().is_empty() {
        return 0.0;
    }

    let mut score: f64 = 1.0;

    // Very short response penalty (absolute floor)
    if response.len() < 20 {
        score -= 0.3;
    }

    // Category-aware length expectations
    let min_expected = match category {
        "test_generation" | "code_review" | "security_scan" | "code_generation" => 200,
        "refactoring" | "frontend_ui" | "backend_api" => 150,
        "documentation" => 100,
        _ => 50,
    };
    if response.len() < min_expected {
        let ratio = response.len() as f64 / min_expected as f64;
        score -= 0.3 * (1.0 - ratio);
    }

    // Output loop detection
    let lines: Vec<&str> = response.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() >= 6 {
        let unique: std::collections::HashSet<&str> = lines.iter().copied().collect();
        if unique.len() * 3 < lines.len() {
            score -= 0.4;
        }
    }

    // High latency penalty (> 30 seconds)
    if latency_ms > 30_000 {
        score -= 0.1;
    }

    score.clamp(0.0, 1.0)
}

/// Warm-start priors for a model based on static heuristic knowledge.
///
/// Returns `(category_str, alpha, beta)` tuples reflecting which categories
/// each model excels at. Unknown pairings use cold-start `Beta(2, 1)`.
fn static_model_priors(model: &ModelChoice) -> Vec<(&'static str, f64, f64)> {
    match model {
        ModelChoice::LocalGlm47Flash => vec![
            ("general", 5.0, 2.0),
            ("test_generation", 5.0, 2.0),
            ("code_review", 5.0, 2.0),
            ("refactoring", 4.0, 2.0),
        ],
        ModelChoice::LocalQwen3 => vec![
            ("code_search", 5.0, 2.0),
            ("documentation", 5.0, 2.0),
            ("backend_api", 5.0, 2.0),
        ],
        ModelChoice::LocalMinistral3B => {
            vec![("security_scan", 3.0, 2.0), ("code_generation", 3.0, 2.0)]
        }
        ModelChoice::LocalMinistral8B => vec![
            ("general", 4.0, 2.0),
            ("code_generation", 4.0, 2.0),
            ("test_generation", 4.0, 2.0),
        ],
        ModelChoice::DeepSeekV32 => vec![("code_generation", 5.0, 2.0), ("backend_api", 4.0, 2.0)],
        ModelChoice::ClaudeSonnet => vec![("frontend_ui", 5.0, 2.0), ("refactoring", 5.0, 2.0)],
        ModelChoice::ClaudeOpus => vec![("test_generation", 5.0, 2.0), ("backend_api", 5.0, 2.0)],
        ModelChoice::GeminiFlash => vec![("frontend_ui", 5.0, 2.0)],
        ModelChoice::GeminiPro => vec![("test_generation", 5.0, 2.0), ("backend_api", 5.0, 2.0)],
        ModelChoice::KimiK2 => vec![("code_generation", 3.0, 2.0)],
        _ => vec![],
    }
}

/// Seed the learning module with warm-start priors for all feasible models
pub async fn seed_model_learning(selector: &ModelSelector, learning: &LearningModule) {
    let feasible = selector.feasible_models();
    for model in &feasible {
        let priors = static_model_priors(model);
        if !priors.is_empty() {
            learning.seed_priors(model.name(), &priors).await;
        }
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
        // Should always have at least one model (Qwen3 fallback)
        assert!(!feasible.is_empty());
    }

    #[tokio::test]
    async fn test_select_adaptive_uses_thompson_sampling() {
        let selector = ModelSelector::with_availability(gemini_only());
        let learning = LearningModule::new();

        // Seed and train: make GeminiFlash very successful for frontend
        use crate::learning::BurstFeedback;
        for _ in 0..20 {
            learning
                .immediate_update(
                    "gemini-flash-latest",
                    &BurstFeedback::success(uuid::Uuid::new_v4(), "frontend_ui".to_string(), 100),
                )
                .await;
        }

        // Make GeminiPro fail for frontend
        for _ in 0..20 {
            learning
                .immediate_update(
                    "gemini-3-pro-preview",
                    &BurstFeedback::failure(uuid::Uuid::new_v4(), "frontend_ui".to_string(), 100),
                )
                .await;
        }

        let classification =
            Classification::new(TaskCategory::FrontendUI, 0.90, "Frontend task".to_string());

        // Adaptive selection should prefer GeminiFlash most of the time
        let mut flash_count = 0;
        for _ in 0..20 {
            let decision = selector
                .select_adaptive(&learning, &classification, 0.0, &[])
                .await
                .unwrap();
            if decision.recommended_model == ModelChoice::GeminiFlash {
                flash_count += 1;
            }
        }
        assert!(
            flash_count > 10,
            "GeminiFlash should be selected most times (got {flash_count}/20)"
        );
    }

    #[tokio::test]
    async fn test_select_adaptive_reasoning_contains_thompson() {
        let selector = ModelSelector::with_availability(gemini_only());
        let learning = LearningModule::new();

        let classification =
            Classification::new(TaskCategory::General, 0.70, "General task".to_string());

        let decision = selector
            .select_adaptive(&learning, &classification, 0.0, &[])
            .await
            .unwrap();
        assert!(decision.reasoning.contains("Thompson Sampling"));
    }

    #[tokio::test]
    async fn test_select_adaptive_budget_excludes_cloud() {
        let selector = ModelSelector::with_availability(gemini_only());
        let learning = LearningModule::new();

        let classification =
            Classification::new(TaskCategory::FrontendUI, 0.90, "Frontend task".to_string());

        // Over budget — should only pick local models
        let decision = selector
            .select_adaptive(&learning, &classification, 0.95, &[])
            .await
            .unwrap();
        assert!(
            decision.recommended_model.is_local(),
            "Over-budget should force local: got {}",
            decision.recommended_model.name()
        );
    }

    #[tokio::test]
    async fn test_select_adaptive_exclusions() {
        let selector = ModelSelector::with_availability(gemini_only());
        let learning = LearningModule::new();

        let classification =
            Classification::new(TaskCategory::General, 0.70, "General task".to_string());

        // Exclude GeminiFlash — should not be selected
        let excluded = vec!["gemini-flash-latest".to_string()];
        let decision = selector
            .select_adaptive(&learning, &classification, 0.0, &excluded)
            .await
            .unwrap();
        assert_ne!(
            decision.recommended_model,
            ModelChoice::GeminiFlash,
            "Excluded model should not be selected"
        );
    }

    #[test]
    fn test_compute_response_quality_empty() {
        assert_eq!(compute_response_quality("", 100, "general"), 0.0);
        assert_eq!(compute_response_quality("   ", 100, "general"), 0.0);
    }

    #[test]
    fn test_compute_response_quality_normal() {
        let quality = compute_response_quality(
            "This is a normal response with useful content.",
            500,
            "general",
        );
        assert!(
            quality > 0.8,
            "Normal response should score high: {quality}"
        );
    }

    #[test]
    fn test_compute_response_quality_loop_detection() {
        let looped =
            "same line\nsame line\nsame line\nsame line\nsame line\nsame line\nsame line\n";
        let quality = compute_response_quality(looped, 100, "general");
        assert!(quality < 0.7, "Looped response should score low: {quality}");
    }

    #[test]
    fn test_compute_response_quality_high_latency() {
        let quality = compute_response_quality("A reasonable response.", 35_000, "general");
        assert!(quality < 1.0, "High latency should reduce score: {quality}");
    }

    #[test]
    fn test_compute_response_quality_category_aware() {
        let short_response = "Fixed the bug.";

        // Same short response, different categories → different scores
        let general_quality = compute_response_quality(short_response, 100, "general");
        let test_gen_quality = compute_response_quality(short_response, 100, "test_generation");
        let code_search_quality = compute_response_quality(short_response, 100, "code_search");

        // test_generation expects 200+ chars, so a 14-char response scores much lower
        assert!(
            test_gen_quality < general_quality,
            "Complex category should penalize short response: test_gen={test_gen_quality}, general={general_quality}"
        );
        assert!(
            code_search_quality >= general_quality,
            "Simple category should not penalize more than general"
        );
    }

    #[test]
    fn test_compute_response_quality_adequate_for_complex() {
        // A decent-length response for a complex task should score well
        let response = "Here is a comprehensive test suite with multiple test cases covering edge cases, error handling, and happy paths. Each test verifies the expected behavior of the function under different conditions.";
        let quality = compute_response_quality(response, 500, "test_generation");
        assert!(
            quality > 0.9,
            "Adequate response for complex task should score high: {quality}"
        );
    }

    #[tokio::test]
    async fn test_seed_model_learning() {
        let selector = ModelSelector::with_availability(gemini_only());
        let learning = LearningModule::new();

        seed_model_learning(&selector, &learning).await;

        // GeminiFlash should have seeded frontend_ui prior
        let stats = learning.get_category_stats("gemini-flash-latest").await;
        let frontend = stats.iter().find(|(k, _, _, _, _)| k == "frontend_ui");
        assert!(
            frontend.is_some(),
            "GeminiFlash should have frontend_ui prior seeded"
        );
        if let Some((_, alpha, _, _, _)) = frontend {
            assert!(*alpha > 4.0, "Seeded alpha should be optimistic: {alpha}");
        }
    }
}
