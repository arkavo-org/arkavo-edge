use crate::classifier::TaskCategory;
use crate::decision::ModelChoice;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod complexity;
mod executor;
mod planner;

pub use complexity::{ComplexityScore, ComplexityScorer};
pub use executor::{ArchitectExecutor, SubtaskResult};
pub use planner::ArchitectPlanner;

/// Represents a decomposed task plan created by the architect phase
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectPlan {
    /// Unique identifier for this plan
    pub id: Uuid,
    /// Original task description from user
    pub original_task: String,
    /// Complexity analysis that triggered architect mode
    pub complexity_score: ComplexityScore,
    /// Ordered list of subtasks to execute
    pub subtasks: Vec<Subtask>,
    /// Estimated cost if Opus handled entire task alone (for savings calculation)
    pub opus_only_estimate_usd: f64,
    /// Estimated total cost with architect mode routing
    pub architect_estimate_usd: f64,
    /// Reasoning/thinking content from the planning model (e.g., DeepSeek V3.2-Speciale)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planning_reasoning: Option<String>,
}

impl ArchitectPlan {
    pub fn new(task: String, complexity: ComplexityScore) -> Self {
        Self {
            id: Uuid::new_v4(),
            original_task: task,
            complexity_score: complexity,
            subtasks: Vec::new(),
            opus_only_estimate_usd: 0.0,
            architect_estimate_usd: 0.0,
            planning_reasoning: None,
        }
    }

    /// Calculate estimated savings percentage
    pub fn estimated_savings_percent(&self) -> f32 {
        if self.opus_only_estimate_usd > 0.0 {
            let savings = self.opus_only_estimate_usd - self.architect_estimate_usd;
            ((savings / self.opus_only_estimate_usd) * 100.0) as f32
        } else {
            0.0
        }
    }

    /// Add a subtask to the plan
    pub fn add_subtask(&mut self, subtask: Subtask) {
        self.subtasks.push(subtask);
    }
}

/// Individual subtask within an architect plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    /// Unique identifier for this subtask
    pub id: Uuid,
    /// Index in execution order (0-based)
    pub index: usize,
    /// Description of what needs to be done
    pub description: String,
    /// Auto-classified category (used for model routing)
    pub category: TaskCategory,
    /// Model assigned based on category
    pub assigned_model: ModelChoice,
    /// Estimated cost for this subtask
    pub estimated_cost_usd: f64,
    /// Indices of subtasks this depends on (must complete first)
    pub dependencies: Vec<usize>,
}

impl Subtask {
    pub fn new(index: usize, description: String, category: TaskCategory) -> Self {
        Self {
            id: Uuid::new_v4(),
            index,
            description,
            category,
            assigned_model: ModelChoice::LocalQwen3, // Default, will be set by selector
            estimated_cost_usd: 0.0,
            dependencies: Vec::new(),
        }
    }

    pub fn with_model(mut self, model: ModelChoice, cost: f64) -> Self {
        self.assigned_model = model;
        self.estimated_cost_usd = cost;
        self
    }

    pub fn with_dependencies(mut self, deps: Vec<usize>) -> Self {
        self.dependencies = deps;
        self
    }
}

/// Result of executing an architect plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectResult {
    /// The plan that was executed
    pub plan: ArchitectPlan,
    /// Results from each subtask
    pub subtask_results: Vec<SubtaskResult>,
    /// Final synthesized response
    pub final_response: String,
    /// Actual total cost incurred
    pub actual_cost_usd: f64,
    /// Actual savings compared to Opus-only estimate
    pub actual_savings_usd: f64,
    /// Whether architect mode was beneficial
    pub was_cost_effective: bool,
}

impl ArchitectResult {
    /// Create a result for when architect mode was skipped (simple task)
    pub fn single_task(response: String, cost: f64) -> Self {
        Self {
            plan: ArchitectPlan::new(String::new(), ComplexityScore::simple()),
            subtask_results: Vec::new(),
            final_response: response,
            actual_cost_usd: cost,
            actual_savings_usd: 0.0,
            was_cost_effective: false,
        }
    }

    /// Calculate actual savings percentage achieved
    pub fn savings_percent(&self) -> f32 {
        if self.plan.opus_only_estimate_usd > 0.0 {
            ((self.actual_savings_usd / self.plan.opus_only_estimate_usd) * 100.0) as f32
        } else {
            0.0
        }
    }

    /// Compute a PlannerScore from subtask results.
    ///
    /// Converts each `SubtaskResult` into a `SubtaskOutcome` with heuristic
    /// quality scoring and failure attribution, then delegates to `PlannerScore::compute`.
    pub fn compute_planner_score(
        &self,
        task_id: Uuid,
        planner_model: &str,
    ) -> crate::learning::PlannerScore {
        use crate::learning::{PlannerScore, SubtaskOutcome};
        use crate::selector_quality::compute_response_quality;

        let outcomes: Vec<SubtaskOutcome> = self
            .subtask_results
            .iter()
            .map(|r| {
                let quality = if r.success {
                    compute_response_quality(
                        &r.response,
                        0, // latency not tracked per-subtask currently
                        &self.plan.subtasks[r.index].category.as_str(),
                    )
                } else {
                    0.0
                };

                let outcome = SubtaskOutcome {
                    subtask_id: r.subtask_id,
                    worker_model: r.model_used.name().to_string(),
                    success: r.success,
                    quality,
                    was_plan_flaw: None,
                };

                if let Some(err) = &r.error {
                    outcome.with_failure_attribution(err)
                } else {
                    outcome.with_failure_attribution("")
                }
            })
            .collect();

        PlannerScore::compute(self.plan.id, task_id, planner_model.to_string(), outcomes)
    }

    /// Compute per-step quality rewards for retrospective credit assignment.
    ///
    /// Returns a `Vec<f64>` with one quality score per subtask result,
    /// suitable for `FinalTaskReport.per_step_rewards`.
    pub fn per_step_rewards(&self) -> Vec<f64> {
        use crate::selector_quality::compute_response_quality;

        self.subtask_results
            .iter()
            .map(|r| {
                if r.success {
                    compute_response_quality(
                        &r.response,
                        0,
                        &self.plan.subtasks[r.index].category.as_str(),
                    )
                } else {
                    0.0
                }
            })
            .collect()
    }
}

/// Cost tracking metadata for architect mode spending records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectCostMetadata {
    /// Whether this spending was in architect mode
    pub architect_mode: bool,
    /// Plan ID this spending belongs to
    pub plan_id: Option<Uuid>,
    /// Phase of execution
    pub phase: ArchitectPhase,
    /// Subtask ID if in execution phase
    pub subtask_id: Option<Uuid>,
    /// What we estimated Opus-only would cost
    pub opus_only_estimate: Option<f64>,
}

/// Phase of architect mode execution
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ArchitectPhase {
    /// Opus is generating the plan
    Planning,
    /// Executing a subtask
    Execution,
    /// Synthesizing final result
    Synthesis,
}

impl Default for ArchitectCostMetadata {
    fn default() -> Self {
        Self {
            architect_mode: false,
            plan_id: None,
            phase: ArchitectPhase::Planning,
            subtask_id: None,
            opus_only_estimate: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use executor::SubtaskResult;

    #[test]
    fn test_architect_plan_savings() {
        let mut plan = ArchitectPlan::new("test task".to_string(), ComplexityScore::simple());
        plan.opus_only_estimate_usd = 0.10;
        plan.architect_estimate_usd = 0.03;

        assert!((plan.estimated_savings_percent() - 70.0).abs() < 0.1);
    }

    #[test]
    fn test_subtask_builder() {
        let subtask = Subtask::new(0, "Build UI".to_string(), TaskCategory::FrontendUI)
            .with_model(ModelChoice::GeminiFlash, 0.005)
            .with_dependencies(vec![]);

        assert_eq!(subtask.index, 0);
        assert_eq!(subtask.assigned_model, ModelChoice::GeminiFlash);
        assert!(subtask.dependencies.is_empty());
    }

    fn make_plan_with_subtasks(count: usize) -> ArchitectPlan {
        let mut plan = ArchitectPlan::new("multi-step task".to_string(), ComplexityScore::simple());
        for i in 0..count {
            plan.add_subtask(
                Subtask::new(i, format!("subtask {i}"), TaskCategory::General)
                    .with_model(ModelChoice::GeminiFlash, 0.01),
            );
        }
        plan
    }

    fn make_result(
        plan: ArchitectPlan,
        successes: &[(usize, &str)],
        failures: &[(usize, &str)],
    ) -> ArchitectResult {
        let mut results = Vec::new();
        for &(idx, response) in successes {
            results.push(SubtaskResult {
                subtask_id: Uuid::new_v4(),
                index: idx,
                model_used: ModelChoice::GeminiFlash,
                response: response.to_string(),
                reasoning_content: None,
                tool_calls: vec![],
                actual_cost_usd: 0.01,
                success: true,
                error: None,
                retry_count: 0,
            });
        }
        for &(idx, error_msg) in failures {
            results.push(SubtaskResult {
                subtask_id: Uuid::new_v4(),
                index: idx,
                model_used: ModelChoice::GeminiFlash,
                response: String::new(),
                reasoning_content: None,
                tool_calls: vec![],
                actual_cost_usd: 0.0,
                success: false,
                error: Some(error_msg.to_string()),
                retry_count: 2,
            });
        }
        // Sort by index so plan.subtasks[r.index] works
        results.sort_by_key(|r| r.index);

        ArchitectResult {
            plan,
            subtask_results: results,
            final_response: "done".to_string(),
            actual_cost_usd: 0.02,
            actual_savings_usd: 0.08,
            was_cost_effective: true,
        }
    }

    #[test]
    fn test_planner_score_all_success() {
        let plan = make_plan_with_subtasks(2);
        let result = make_result(
            plan,
            &[
                (
                    0,
                    "A full implementation of the UI component with proper styling and tests",
                ),
                (
                    1,
                    "Complete backend API with validation, error handling, and documentation",
                ),
            ],
            &[],
        );

        let score = result.compute_planner_score(Uuid::new_v4(), "deepseek-planner");
        assert_eq!(score.subtask_count, 2);
        assert!((score.coverage - 1.0).abs() < 0.01);
        assert!(score.overall_score > 0.8, "score={}", score.overall_score);
    }

    #[test]
    fn test_planner_score_with_plan_flaw() {
        let plan = make_plan_with_subtasks(2);
        let result = make_result(
            plan,
            &[(
                0,
                "Implemented the full feature with tests and documentation",
            )],
            &[(1, "Error: unknown tool 'nonexistent_tool'")],
        );

        let score = result.compute_planner_score(Uuid::new_v4(), "deepseek-planner");
        assert_eq!(score.subtask_count, 2);
        // Plan flaw should be detected and penalize the score
        assert!(
            score.subtask_outcomes[1].was_plan_flaw == Some(true),
            "unknown tool should be attributed as plan flaw"
        );
        assert!(
            score.overall_score < 0.5,
            "plan flaw penalty: {}",
            score.overall_score
        );
    }

    #[test]
    fn test_per_step_rewards_mixed() {
        let plan = make_plan_with_subtasks(3);
        let result = make_result(
            plan,
            &[
                (
                    0,
                    "Good response with enough content to score well in quality heuristic",
                ),
                (
                    2,
                    "Another detailed response covering the implementation thoroughly",
                ),
            ],
            &[(1, "timeout after 30s")],
        );

        let rewards = result.per_step_rewards();
        assert_eq!(rewards.len(), 3);
        assert!(rewards[0] > 0.5, "successful subtask should score >0.5");
        assert_eq!(rewards[1], 0.0, "failed subtask should score 0.0");
        assert!(rewards[2] > 0.5, "successful subtask should score >0.5");
    }

    #[test]
    fn test_per_step_rewards_empty_plan() {
        let plan = make_plan_with_subtasks(0);
        let result = ArchitectResult::single_task("done".to_string(), 0.01);
        assert!(result.per_step_rewards().is_empty());
        let _ = plan; // suppress unused warning
    }
}
