use crate::Result;
use crate::classifier::{Classification, TaskCategory};
use crate::decision::{ModelChoice, RoutingDecision};
use crate::learning::{DecisionTrace, LearningModule};
use crate::selector::ModelSelector;

impl ModelSelector {
    pub(crate) fn explain_selection(
        &self,
        model: &ModelChoice,
        classification: &Classification,
    ) -> String {
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
            (TaskCategory::GameSimulation, _) => {
                "Game/simulation task: Local model for domain-specific reasoning"
            }
            (TaskCategory::General, _) => "General task: Local model for efficiency",
        };

        let model_benefit = match model {
            ModelChoice::GeminiFlash => "Fast (3s), cost-effective ($0.003-0.006)",
            ModelChoice::Gemini35Flash => {
                "Frontier reasoning at ~280 tok/s (2s), 1M context, multimodal ($1.50/$9.00 per M)"
            }
            ModelChoice::GeminiPro => "Highest quality, comprehensive output ($0.009)",
            ModelChoice::ClaudeSonnet => "Fast (5s), excellent quality ($0.018-0.045)",
            ModelChoice::ClaudeOpus => "Premium quality, complex reasoning ($0.090-0.225)",
            ModelChoice::LocalQwen3 => "Ultra-fast (<1s), zero cost, TØRG-compatible",
            ModelChoice::LocalMinistral3B => "Fast (2s), zero cost, TØRG-compatible",
            ModelChoice::LocalMinistral8B => "High quality (4s), zero cost, TØRG-compatible",
            ModelChoice::LocalQwen35_9B => "9B dense reasoning (4s), zero cost, good quality",
            ModelChoice::LocalQwen35_27B => "27B dense reasoning (10s), zero cost, high quality",
            ModelChoice::LocalQwen36A3B => {
                "35B MoE (6s), zero cost, vision, 3B active, 262K context"
            }
            ModelChoice::LocalGlm47Flash => "30B MoE reasoning (8s), zero cost, excellent quality",
            ModelChoice::LocalGemma4E2B => "Gemma 4 edge (1s), zero cost, vision, 2.3B active",
            ModelChoice::LocalGemma4E4B => "Gemma 4 edge (2s), zero cost, vision, 4.5B active",
            ModelChoice::LocalGemma4_26B => {
                "Gemma 4 MoE (8s), zero cost, vision, 4B active/26B total, 128 experts"
            }
            ModelChoice::LocalGemma4_31B => {
                "Gemma 4 dense (12s), zero cost, vision, 31B params, strong reasoning"
            }
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
            TaskCategory::FrontendUI => {
                if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }
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
            TaskCategory::Documentation => {
                if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else {
                    ModelChoice::LocalQwen3
                }
            }
            TaskCategory::Refactoring => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }
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
            TaskCategory::CodeSearch => ModelChoice::LocalQwen3,
            TaskCategory::VisionAnalysis => {
                if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }
            TaskCategory::GameSimulation | TaskCategory::General => ModelChoice::LocalQwen3,
            #[allow(unreachable_patterns)]
            _ => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::Gemini35Flash
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

            decision.trace = DecisionTrace::budget_constrained(
                classification.category,
                f64::from(classification.confidence),
                local_fallback.name(),
                budget_usage_percent,
            );
            decision.recommended_model = local_fallback;
            decision.estimated_cost_usd = 0.0;
        }

        Ok(decision)
    }

    /// Select model using Thompson Sampling from the learning module.
    ///
    /// Budget and exclusions are applied to the feasible set *before* sampling,
    /// so Thompson Sampling never picks a model that will be rejected downstream.
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

        let excluded_names: Vec<String> = excluded.to_vec();

        if feasible.len() == 1 {
            let model = feasible
                .into_iter()
                .next()
                .unwrap_or(ModelChoice::LocalQwen3);
            let reasoning = format!("Single feasible model: {}", model.name());
            let trace = DecisionTrace::single_feasible(
                classification.category,
                f64::from(classification.confidence),
                model.name(),
                budget_usage,
                excluded_names,
            );
            return Ok(RoutingDecision::with_trace(
                model,
                classification.category,
                classification.confidence,
                reasoning,
                trace,
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
            let trace = DecisionTrace::thompson(
                classification.category,
                f64::from(classification.confidence),
                model_ids,
                ranked.clone(),
                model.name(),
                0,
                budget_usage,
                excluded_names,
            );
            Ok(RoutingDecision::with_trace(
                model,
                classification.category,
                classification.confidence,
                reasoning,
                trace,
            ))
        } else {
            self.select(classification, "")
        }
    }
}

/// Heuristic quality score for a model response (0.0 to 1.0, no LLM call).
///
/// Category-aware: complex task types expect longer, more detailed responses.
pub fn compute_response_quality(
    response: &str,
    latency_ms: u64,
    category: &str,
    tool_call_count: usize,
) -> f64 {
    if response.trim().is_empty() {
        if tool_call_count > 0 {
            // Degenerate output: model entered a repetition loop
            if tool_call_count > 10 {
                return 0.0;
            }
            // Tool-only responses (function calls, no text) are valid
            let base = 0.7;
            let bonus = (tool_call_count as f64 * 0.1).min(0.3);
            return (base + bonus).min(1.0);
        }
        return 0.0;
    }

    let mut score: f64 = 1.0;

    // Error responses should not score high
    let lower = response.to_lowercase();
    if lower.starts_with("error:")
        || lower.contains("tool execution error")
        || lower.contains("serialization error")
        || lower.contains("failed to execute")
    {
        score -= 0.6;
    }

    if response.len() < 20 {
        score -= 0.3;
    }

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

    let lines: Vec<&str> = response.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() >= 6 {
        let unique: std::collections::HashSet<&str> = lines.iter().copied().collect();
        if unique.len() * 3 < lines.len() {
            score -= 0.4;
        }
    }

    if latency_ms > 30_000 {
        score -= 0.1;
    }

    // Degenerate tool call count penalty (even with text content)
    if tool_call_count > 10 {
        score -= 0.8;
    }

    score.clamp(0.0, 1.0)
}

/// Warm-start priors for a model based on static heuristic knowledge.
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
        ModelChoice::LocalQwen35_27B => vec![
            ("general", 5.0, 2.0),
            ("code_generation", 5.0, 2.0),
            ("test_generation", 4.0, 2.0),
            ("refactoring", 4.0, 2.0),
        ],
        // Qwen 3.6 uses qwen3_coder tool-call format (XML); Fence prompt causes
        // hybrid output (fence open + XML params) that breaks parsing (6/8 vs 8/8).
        // All four formats seeded so sample_format doesn't fall back to the
        // cold-start Beta(2,1) prior for json/python and outsample XML.
        ModelChoice::LocalQwen36A3B => vec![
            ("format:xml", 20.0, 1.0),
            ("format:fence", 1.0, 10.0),
            ("format:json", 1.0, 5.0),
            ("format:python", 1.0, 5.0),
            ("general", 5.0, 2.0),
            ("code_generation", 5.0, 2.0),
            ("vision_analysis", 4.0, 2.0),
        ],
        ModelChoice::DeepSeekV32 => {
            vec![("code_generation", 5.0, 2.0), ("backend_api", 4.0, 2.0)]
        }
        ModelChoice::ClaudeSonnet => {
            vec![("frontend_ui", 5.0, 2.0), ("refactoring", 5.0, 2.0)]
        }
        ModelChoice::ClaudeOpus => {
            vec![("test_generation", 5.0, 2.0), ("backend_api", 5.0, 2.0)]
        }
        ModelChoice::GeminiFlash => vec![("frontend_ui", 5.0, 2.0)],
        // Gemini 3.5 Flash: pro-tier reasoning + agentic; warm-start for code & agent tasks.
        ModelChoice::Gemini35Flash => vec![
            ("frontend_ui", 5.0, 2.0),
            ("code_generation", 5.0, 2.0),
            ("refactoring", 4.0, 2.0),
            ("vision_analysis", 5.0, 2.0),
        ],
        ModelChoice::GeminiPro => {
            vec![("test_generation", 5.0, 2.0), ("backend_api", 5.0, 2.0)]
        }
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
    use crate::classifier::Classification;
    use crate::learning::BurstFeedback;
    use crate::selector::ProviderAvailability;

    fn gemini_only() -> ProviderAvailability {
        ProviderAvailability {
            gemini: true,
            anthropic: false,
            deepseek: false,
            kimi: false,
        }
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
    async fn test_select_adaptive_uses_thompson_sampling() {
        let selector = ModelSelector::with_availability(gemini_only());
        let learning = LearningModule::new();

        // Feed positive evidence for both Flash variants and negative for Pro,
        // so Thompson Sampling should converge on one of the two Flash models.
        for _ in 0..20 {
            learning
                .immediate_update(
                    "gemini-flash-latest",
                    &BurstFeedback::success(uuid::Uuid::new_v4(), "frontend_ui".to_string(), 100),
                )
                .await;
            learning
                .immediate_update(
                    "gemini-3.5-flash",
                    &BurstFeedback::success(uuid::Uuid::new_v4(), "frontend_ui".to_string(), 100),
                )
                .await;
        }
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
        let mut flash_count = 0;
        for _ in 0..20 {
            let decision = selector
                .select_adaptive(&learning, &classification, 0.0, &[])
                .await
                .unwrap();
            if matches!(
                decision.recommended_model,
                ModelChoice::GeminiFlash | ModelChoice::Gemini35Flash
            ) {
                flash_count += 1;
            }
        }
        assert!(
            flash_count > 10,
            "A Flash-family model should be selected most times (got {flash_count}/20)"
        );
    }

    #[tokio::test]
    async fn test_select_adaptive_reasoning_contains_thompson() {
        // Need both Gemini and Anthropic so feasible set has >1 model
        // (single-model path skips Thompson Sampling)
        let selector = ModelSelector::with_availability(ProviderAvailability {
            gemini: true,
            anthropic: true,
            deepseek: false,
            kimi: false,
        });
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
        assert_eq!(compute_response_quality("", 100, "general", 0), 0.0);
        assert_eq!(compute_response_quality("   ", 100, "general", 0), 0.0);
    }

    #[test]
    fn test_compute_response_quality_tool_only() {
        // Tool-only responses (empty text, function calls present) should score well
        let q1 = compute_response_quality("", 100, "general", 1);
        assert!(q1 >= 0.7, "Single tool call should score >= 0.7: {q1}");

        let q3 = compute_response_quality("", 100, "general", 3);
        assert!(q3 >= 0.9, "Three tool calls should score >= 0.9: {q3}");
        assert!(q3 <= 1.0, "Quality should not exceed 1.0: {q3}");

        // Whitespace-only with tool calls should also score well
        let qw = compute_response_quality("   ", 100, "general", 2);
        assert!(qw >= 0.8, "Whitespace + 2 tool calls: {qw}");
    }

    #[test]
    fn test_compute_response_quality_normal() {
        let quality = compute_response_quality(
            "This is a normal response with useful content.",
            500,
            "general",
            0,
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
        let quality = compute_response_quality(looped, 100, "general", 0);
        assert!(quality < 0.7, "Looped response should score low: {quality}");
    }

    #[test]
    fn test_compute_response_quality_high_latency() {
        let quality = compute_response_quality("A reasonable response.", 35_000, "general", 0);
        assert!(quality < 1.0, "High latency should reduce score: {quality}");
    }

    #[test]
    fn test_compute_response_quality_category_aware() {
        let short_response = "Fixed the bug.";
        let general_quality = compute_response_quality(short_response, 100, "general", 0);
        let test_gen_quality = compute_response_quality(short_response, 100, "test_generation", 0);
        let code_search_quality = compute_response_quality(short_response, 100, "code_search", 0);
        assert!(
            test_gen_quality < general_quality,
            "Complex category should penalize: test_gen={test_gen_quality}, general={general_quality}"
        );
        assert!(
            code_search_quality >= general_quality,
            "Simple category should not penalize more than general"
        );
    }

    #[test]
    fn test_compute_response_quality_adequate_for_complex() {
        let response = "Here is a comprehensive test suite with multiple test cases covering edge cases, error handling, and happy paths. Each test verifies the expected behavior of the function under different conditions.";
        let quality = compute_response_quality(response, 500, "test_generation", 0);
        assert!(
            quality > 0.9,
            "Adequate response should score high: {quality}"
        );
    }

    #[test]
    fn test_compute_response_quality_error_detection() {
        let error_resp = "Error: Tool execution error: Serialization error: data did not match any variant of untagged enum";
        let quality = compute_response_quality(error_resp, 500, "general", 0);
        assert!(quality < 0.5, "Error response should score low: {quality}");

        let tool_error = "Something happened then tool execution error occurred in the process";
        let quality2 = compute_response_quality(tool_error, 500, "general", 0);
        assert!(
            quality2 < 0.5,
            "Tool error response should score low: {quality2}"
        );

        let failed = "failed to execute the requested tool call";
        let quality3 = compute_response_quality(failed, 500, "general", 0);
        assert!(
            quality3 < 0.5,
            "Failed execution should score low: {quality3}"
        );
    }

    #[test]
    fn quality_penalizes_degenerate_tool_call_count() {
        // 125 tool calls with empty response = degenerate output loop
        let score = compute_response_quality("", 55_000, "general", 125);
        assert!(
            score < 0.1,
            "125 tool calls should score near 0, got {score}"
        );
    }

    #[test]
    fn quality_rewards_moderate_tool_call_count() {
        // 3 tool calls = healthy batch
        let score = compute_response_quality("", 10_000, "general", 3);
        assert!(score >= 0.7, "3 tool calls should score >=0.7, got {score}");
    }

    #[test]
    fn quality_penalizes_borderline_tool_call_count() {
        // 15 tool calls = suspicious but less severe
        let score = compute_response_quality("", 20_000, "general", 15);
        assert!(score < 0.3, "15 tool calls should score low, got {score}");
    }

    #[tokio::test]
    async fn test_qwen36_format_prior_prefers_xml() {
        use crate::learning::ToolCallFormat;

        let learning = LearningModule::new();
        let agent = ModelChoice::LocalQwen36A3B.name();
        let priors = static_model_priors(&ModelChoice::LocalQwen36A3B);
        let prior_pairs: Vec<(&str, f64, f64)> =
            priors.iter().map(|(k, a, b)| (*k, *a, *b)).collect();
        learning.seed_priors(agent, &prior_pairs).await;

        let mut xml_wins = 0;
        for _ in 0..60 {
            let (format, _) = learning.sample_format(agent).await;
            if format == ToolCallFormat::Xml {
                xml_wins += 1;
            }
        }
        assert!(
            xml_wins >= 40,
            "XML should dominate format sampling for Qwen 3.6 (got {xml_wins}/60)"
        );
    }

    #[test]
    fn test_all_seeded_format_priors_parse() {
        use crate::learning::ToolCallFormat;
        for model in ModelChoice::ALL_LOCAL {
            for (key, _, _) in static_model_priors(model) {
                if key.starts_with("format:") {
                    assert!(
                        ToolCallFormat::from_category_key(key).is_some(),
                        "Invalid format category key {key:?} seeded for {model:?}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn test_seed_model_learning() {
        let selector = ModelSelector::with_availability(gemini_only());
        let learning = LearningModule::new();
        seed_model_learning(&selector, &learning).await;
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
