use super::{ArchitectPlan, ComplexityScore, Subtask};
use crate::classifier::TaskCategory;
use crate::decision::ModelChoice;
use crate::selector::ProviderAvailability;
use crate::{Error, Result};
use arkavo_llm::{Message, Provider};
use serde::Deserialize;

/// Creates execution plans by decomposing complex tasks using Opus
pub struct ArchitectPlanner {
    availability: ProviderAvailability,
}

impl ArchitectPlanner {
    pub fn new() -> Self {
        Self {
            availability: ProviderAvailability::from_env(),
        }
    }

    /// Create a plan using Opus (or best available model) to decompose the task
    pub async fn create_plan(
        &self,
        task: &str,
        complexity: ComplexityScore,
    ) -> Result<ArchitectPlan> {
        let provider = self.get_planning_provider()?;

        let prompt = self.build_planning_prompt(task);
        let messages = vec![Message::user(prompt)];

        // Use complete_with_tools to get ProviderResponse with reasoning_content
        let response = provider
            .complete_with_tools(messages, None, None)
            .await
            .map_err(|e| Error::ModelExecution(format!("Planning phase failed: {e}")))?;

        let mut plan = self.parse_plan_response(task, &response.content, complexity)?;

        // Capture reasoning from thinking models (e.g., DeepSeek V3.2-Speciale)
        plan.planning_reasoning = response.reasoning_content;

        // Calculate cost estimates
        self.estimate_costs(&mut plan);

        Ok(plan)
    }

    fn get_planning_provider(&self) -> Result<Box<dyn Provider>> {
        // Prefer Anthropic Opus for planning (highest quality)
        if self.availability.anthropic {
            use arkavo_llm::providers::anthropic::AnthropicProvider;
            if let Ok(provider) = AnthropicProvider::from_env() {
                return Ok(Box::new(provider));
            }
        }

        // DeepSeek V3.2-Speciale is excellent for planning (reasoning-only, cost-effective)
        #[cfg(feature = "deepseek")]
        if self.availability.deepseek {
            use arkavo_llm::DeepSeekProvider;
            if let Ok(provider) = DeepSeekProvider::v32_speciale() {
                return Ok(Box::new(provider));
            }
        }

        // Fallback to Gemini Pro
        if self.availability.gemini
            && let Ok(provider) = arkavo_llm::GeminiProvider::new()
        {
            return Ok(Box::new(provider));
        }

        Err(Error::ModelExecution(
            "No planning model available. Set ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, or GEMINI_API_KEY.".to_string(),
        ))
    }

    fn build_planning_prompt(&self, task: &str) -> String {
        format!(
            r#"You are an expert software architect. Analyze this task and break it into concrete subtasks.

Task: {task}

For each subtask, specify:
1. A clear description of what needs to be done
2. The category (one of: frontend_ui, backend_api, test_generation, documentation, security_scan, refactoring, code_generation)
3. Dependencies (indices of subtasks that must complete first, 0-indexed)

Respond with ONLY valid JSON in this exact format:
{{
  "subtasks": [
    {{
      "description": "Brief description of the subtask",
      "category": "category_name",
      "dependencies": []
    }}
  ]
}}

Guidelines:
- Keep subtasks focused and atomic
- Order subtasks logically (dependencies first)
- Use 3-8 subtasks for most tasks
- Backend tasks should precede frontend tasks that depend on them
- Tests should come after the code they test"#
        )
    }

    fn parse_plan_response(
        &self,
        original_task: &str,
        response: &str,
        complexity: ComplexityScore,
    ) -> Result<ArchitectPlan> {
        // Try to extract JSON from the response
        let json_str = self.extract_json(response)?;

        #[derive(Deserialize)]
        struct PlanResponse {
            subtasks: Vec<SubtaskResponse>,
        }

        #[derive(Deserialize)]
        struct SubtaskResponse {
            description: String,
            category: String,
            #[serde(default)]
            dependencies: Vec<usize>,
        }

        let parsed: PlanResponse = serde_json::from_str(&json_str)
            .map_err(|e| Error::Classification(format!("Failed to parse plan JSON: {e}")))?;

        if parsed.subtasks.is_empty() {
            return Err(Error::Classification(
                "Plan contains no subtasks".to_string(),
            ));
        }

        let mut plan = ArchitectPlan::new(original_task.to_string(), complexity);

        for (index, subtask_resp) in parsed.subtasks.iter().enumerate() {
            let category = TaskCategory::from_string(&subtask_resp.category);
            let model = self.select_model_for_category(category);
            let cost = self.estimate_subtask_cost(&model, category);

            let subtask = Subtask::new(index, subtask_resp.description.clone(), category)
                .with_model(model, cost)
                .with_dependencies(subtask_resp.dependencies.clone());

            plan.add_subtask(subtask);
        }

        Ok(plan)
    }

    fn extract_json(&self, response: &str) -> Result<String> {
        // Try to find JSON object in the response
        if let Some(start) = response.find('{')
            && let Some(end) = response.rfind('}')
        {
            return Ok(response[start..=end].to_string());
        }

        // If no JSON found, return error
        Err(Error::Classification(
            "No valid JSON found in planning response".to_string(),
        ))
    }

    /// Select the best model for a subtask category
    fn select_model_for_category(&self, category: TaskCategory) -> ModelChoice {
        match category {
            // Frontend tasks: Use cheaper, fast models
            TaskCategory::FrontendUI => {
                if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // Backend/Security/Tests/Review: Use more capable models
            TaskCategory::BackendAPI
            | TaskCategory::SecurityScan
            | TaskCategory::TestGeneration
            | TaskCategory::CodeReview => {
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
                    ModelChoice::Gemini35Flash
                } else {
                    ModelChoice::LocalQwen3
                }
            }

            // Refactoring: Use balanced models
            TaskCategory::Refactoring | TaskCategory::CodeGeneration => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // Code search: Local model is sufficient
            TaskCategory::CodeSearch => ModelChoice::LocalQwen3,

            // Vision: Needs multimodal
            TaskCategory::VisionAnalysis => {
                if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // Game/simulation and general: Use balanced default
            TaskCategory::GameSimulation | TaskCategory::General => {
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

    fn estimate_subtask_cost(&self, model: &ModelChoice, category: TaskCategory) -> f64 {
        let token_estimate = category.estimated_tokens();

        match model {
            ModelChoice::GeminiFlash => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 0.30;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 2.50;
                input_cost + output_cost
            }
            ModelChoice::Gemini35Flash => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 1.50;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 9.00;
                input_cost + output_cost
            }
            ModelChoice::GeminiPro => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 1.25;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 5.00;
                input_cost + output_cost
            }
            ModelChoice::ClaudeSonnet => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 3.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 15.00;
                input_cost + output_cost
            }
            ModelChoice::ClaudeOpus => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 15.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 75.00;
                input_cost + output_cost
            }
            _ => 0.0, // Local models are free
        }
    }

    fn estimate_costs(&self, plan: &mut ArchitectPlan) {
        // Calculate architect mode total cost
        let planning_cost = 0.015; // ~200 output tokens from Opus at $75/1M
        let execution_cost: f64 = plan.subtasks.iter().map(|s| s.estimated_cost_usd).sum();
        plan.architect_estimate_usd = planning_cost + execution_cost;

        // Calculate Opus-only estimate
        let total_output_tokens: u32 = plan
            .subtasks
            .iter()
            .map(|s| s.category.estimated_tokens().output)
            .sum();
        let total_input_tokens = total_output_tokens / 3;
        let input_cost = (total_input_tokens as f64 / 1_000_000.0) * 15.00;
        let output_cost = (total_output_tokens as f64 / 1_000_000.0) * 75.00;
        plan.opus_only_estimate_usd = input_cost + output_cost;
    }
}

impl Default for ArchitectPlanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json() {
        let planner = ArchitectPlanner::new();

        let response = r#"Here is the plan:
        {"subtasks": [{"description": "test", "category": "frontend_ui", "dependencies": []}]}
        That's the plan."#;

        let json = planner.extract_json(response).unwrap();
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
    }

    #[test]
    fn test_model_selection_frontend() {
        let planner = ArchitectPlanner::new();
        let model = planner.select_model_for_category(TaskCategory::FrontendUI);

        // Should prefer cheaper models for frontend
        assert!(matches!(
            model,
            ModelChoice::GeminiFlash
                | ModelChoice::Gemini35Flash
                | ModelChoice::ClaudeSonnet
                | ModelChoice::LocalMinistral3B
        ));
    }

    #[test]
    fn test_model_selection_backend() {
        let planner = ArchitectPlanner::new();
        let model = planner.select_model_for_category(TaskCategory::BackendAPI);

        // Should prefer capable models for backend
        assert!(matches!(
            model,
            ModelChoice::ClaudeOpus | ModelChoice::GeminiPro | ModelChoice::LocalMinistral8B
        ));
    }
}
