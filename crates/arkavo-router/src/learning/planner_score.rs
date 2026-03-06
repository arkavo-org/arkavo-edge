//! Planner scoring for separating plan quality from execution quality
//!
//! Tracks how well planners decompose tasks vs how well workers execute subtasks.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Score for a planner's decomposition quality
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerScore {
    /// Plan identifier
    pub plan_id: Uuid,
    /// Task this plan was for
    pub task_id: Uuid,
    /// Which model did the planning
    pub planner_model: String,
    /// Number of subtasks in the plan
    pub subtask_count: usize,
    /// Outcomes for each subtask
    pub subtask_outcomes: Vec<SubtaskOutcome>,
    /// Heuristic: success_rate * coverage
    pub decomposition_quality: f64,
    /// Fraction of subtasks that produced results
    pub coverage: f64,
    /// Overall plan score
    pub overall_score: f64,
}

impl PlannerScore {
    /// Compute a planner score from subtask outcomes
    pub fn compute(
        plan_id: Uuid,
        task_id: Uuid,
        planner_model: String,
        outcomes: Vec<SubtaskOutcome>,
    ) -> Self {
        let subtask_count = outcomes.len();
        if subtask_count == 0 {
            return Self {
                plan_id,
                task_id,
                planner_model,
                subtask_count: 0,
                subtask_outcomes: vec![],
                decomposition_quality: 0.0,
                coverage: 0.0,
                overall_score: 0.0,
            };
        }

        let success_count = outcomes.iter().filter(|o| o.success).count();
        let success_rate = success_count as f64 / subtask_count as f64;

        let produced_results = outcomes.iter().filter(|o| o.quality > 0.0).count();
        let coverage = produced_results as f64 / subtask_count as f64;

        let decomposition_quality = success_rate * coverage;

        // Penalize plans where failures were attributed to plan flaws
        let plan_flaw_count = outcomes
            .iter()
            .filter(|o| o.was_plan_flaw == Some(true))
            .count();
        let plan_flaw_penalty = plan_flaw_count as f64 / subtask_count as f64;

        let overall_score = plan_flaw_penalty
            .mul_add(-0.5, decomposition_quality)
            .clamp(0.0, 1.0);

        Self {
            plan_id,
            task_id,
            planner_model,
            subtask_count,
            subtask_outcomes: outcomes,
            decomposition_quality,
            coverage,
            overall_score,
        }
    }
}

/// Outcome of a single subtask within a plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskOutcome {
    /// Subtask identifier
    pub subtask_id: Uuid,
    /// Which model executed this subtask
    pub worker_model: String,
    /// Whether the subtask succeeded
    pub success: bool,
    /// Quality of the subtask result (0.0-1.0)
    pub quality: f64,
    /// None = attribution unresolved. Some(true) = plan flaw. Some(false) = execution flaw.
    /// Populated by heuristic for clear-cut cases only.
    pub was_plan_flaw: Option<bool>,
}

impl SubtaskOutcome {
    /// Heuristic: assign `was_plan_flaw` based on failure mode
    pub fn with_failure_attribution(mut self, error_msg: &str) -> Self {
        if self.success {
            self.was_plan_flaw = Some(false);
            return self;
        }
        let lower = error_msg.to_lowercase();
        if lower.contains("unknown tool")
            || lower.contains("tool not available")
            || lower.contains("tool not found")
        {
            self.was_plan_flaw = Some(true);
        }
        // Timeouts are ambiguous
        // Other failures leave was_plan_flaw as None
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planner_score_all_success() {
        let outcomes = vec![
            SubtaskOutcome {
                subtask_id: Uuid::new_v4(),
                worker_model: "model-a".into(),
                success: true,
                quality: 0.9,
                was_plan_flaw: Some(false),
            },
            SubtaskOutcome {
                subtask_id: Uuid::new_v4(),
                worker_model: "model-b".into(),
                success: true,
                quality: 0.8,
                was_plan_flaw: Some(false),
            },
        ];

        let score =
            PlannerScore::compute(Uuid::new_v4(), Uuid::new_v4(), "planner".into(), outcomes);

        assert_eq!(score.subtask_count, 2);
        assert!((score.coverage - 1.0).abs() < 0.01);
        assert!((score.decomposition_quality - 1.0).abs() < 0.01);
        assert!(score.overall_score > 0.9);
    }

    #[test]
    fn test_planner_score_with_plan_flaw() {
        let outcomes = vec![
            SubtaskOutcome {
                subtask_id: Uuid::new_v4(),
                worker_model: "model-a".into(),
                success: true,
                quality: 0.9,
                was_plan_flaw: Some(false),
            },
            SubtaskOutcome {
                subtask_id: Uuid::new_v4(),
                worker_model: "model-b".into(),
                success: false,
                quality: 0.0,
                was_plan_flaw: Some(true), // Plan referenced nonexistent tool
            },
        ];

        let score =
            PlannerScore::compute(Uuid::new_v4(), Uuid::new_v4(), "planner".into(), outcomes);

        // One plan flaw should penalize the score
        assert!(
            score.overall_score < 0.5,
            "plan flaw should penalize: {}",
            score.overall_score
        );
    }

    #[test]
    fn test_planner_score_empty() {
        let score = PlannerScore::compute(Uuid::new_v4(), Uuid::new_v4(), "planner".into(), vec![]);
        assert_eq!(score.subtask_count, 0);
        assert_eq!(score.overall_score, 0.0);
    }

    #[test]
    fn test_subtask_failure_attribution_unknown_tool() {
        let outcome = SubtaskOutcome {
            subtask_id: Uuid::new_v4(),
            worker_model: "model-a".into(),
            success: false,
            quality: 0.0,
            was_plan_flaw: None,
        }
        .with_failure_attribution("Error: unknown tool 'read_file'");

        assert_eq!(outcome.was_plan_flaw, Some(true));
    }

    #[test]
    fn test_subtask_failure_attribution_timeout() {
        let outcome = SubtaskOutcome {
            subtask_id: Uuid::new_v4(),
            worker_model: "model-a".into(),
            success: false,
            quality: 0.0,
            was_plan_flaw: None,
        }
        .with_failure_attribution("Request timed out after 30s");

        // Timeout is ambiguous - should remain None
        assert_eq!(outcome.was_plan_flaw, None);
    }

    #[test]
    fn test_subtask_success_attribution() {
        let outcome = SubtaskOutcome {
            subtask_id: Uuid::new_v4(),
            worker_model: "model-a".into(),
            success: true,
            quality: 0.9,
            was_plan_flaw: None,
        }
        .with_failure_attribution("");

        assert_eq!(outcome.was_plan_flaw, Some(false));
    }

    #[test]
    fn test_planner_score_mixed_results() {
        let outcomes = vec![
            SubtaskOutcome {
                subtask_id: Uuid::new_v4(),
                worker_model: "model-a".into(),
                success: true,
                quality: 0.9,
                was_plan_flaw: Some(false),
            },
            SubtaskOutcome {
                subtask_id: Uuid::new_v4(),
                worker_model: "model-b".into(),
                success: false,
                quality: 0.0,
                was_plan_flaw: None, // Unresolved attribution
            },
            SubtaskOutcome {
                subtask_id: Uuid::new_v4(),
                worker_model: "model-c".into(),
                success: true,
                quality: 0.7,
                was_plan_flaw: Some(false),
            },
        ];

        let score =
            PlannerScore::compute(Uuid::new_v4(), Uuid::new_v4(), "planner".into(), outcomes);

        // 2/3 success rate, 2/3 coverage (one has quality=0)
        let expected_quality = (2.0 / 3.0) * (2.0 / 3.0);
        assert!(
            (score.decomposition_quality - expected_quality).abs() < 0.01,
            "quality={}, expected={}",
            score.decomposition_quality,
            expected_quality
        );
        // No plan flaws attributed, so no penalty
        assert!(
            (score.overall_score - expected_quality).abs() < 0.01,
            "overall={}",
            score.overall_score
        );
    }
}
