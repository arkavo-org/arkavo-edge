//! Continuous signal bridge for Thompson Sampling
//!
//! Converts graded rubric evaluations and contract negotiation
//! outcomes into Thompson Sampling beta distribution updates.

use super::feedback_types::BurstFeedback;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Outcome of contract negotiation, for routing prior updates
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ContractOutcome {
    /// Contract accepted on first submission
    FirstRound,
    /// Contract needed revision before approval
    Revised,
    /// Contract failed negotiation, escalated to Planner
    Escalated,
}

impl ContractOutcome {
    /// Map to Thompson Sampling beta distribution update (alpha_add, beta_add).
    ///
    /// - FirstRound: strong positive signal (operator scoped work accurately)
    /// - Revised: neutral (operator needed guidance but converged)
    /// - Escalated: negative signal (operator couldn't align with critic)
    pub fn to_beta_update(&self) -> (f64, f64) {
        match self {
            Self::FirstRound => (1.5, 0.0),
            Self::Revised => (0.5, 0.5),
            Self::Escalated => (0.0, 1.5),
        }
    }
}

/// Convert a rubric composite score into a BurstFeedback for the learning module.
///
/// The existing `LearningModule::immediate_update()` handles `quality_score: Option<f64>`
/// via `apply_fractional_update()`, so this bridges rubric scoring into the
/// existing learning path with no changes to AgentUtility.
pub fn rubric_to_feedback(
    composite: f64,
    task_category: &str,
    burst_id: Uuid,
    latency_ms: u64,
) -> BurstFeedback {
    let success = composite >= 0.5;
    let fb = if success {
        BurstFeedback::success(burst_id, task_category.to_string(), latency_ms)
    } else {
        BurstFeedback::failure(burst_id, task_category.to_string(), latency_ms)
    };
    fb.with_quality(composite)
}

/// Convert a contract outcome into a BurstFeedback for the learning module.
pub fn contract_outcome_to_feedback(
    outcome: ContractOutcome,
    task_category: &str,
    burst_id: Uuid,
    latency_ms: u64,
) -> BurstFeedback {
    let (alpha, beta) = outcome.to_beta_update();
    let quality = alpha / (alpha + beta + f64::EPSILON);
    let success = matches!(
        outcome,
        ContractOutcome::FirstRound | ContractOutcome::Revised
    );
    let fb = if success {
        BurstFeedback::success(burst_id, task_category.to_string(), latency_ms)
    } else {
        BurstFeedback::failure(burst_id, task_category.to_string(), latency_ms)
    };
    fb.with_quality(quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_round_beta_update() {
        let (alpha, beta) = ContractOutcome::FirstRound.to_beta_update();
        assert_eq!(alpha, 1.5);
        assert_eq!(beta, 0.0);
    }

    #[test]
    fn test_revised_beta_update() {
        let (alpha, beta) = ContractOutcome::Revised.to_beta_update();
        assert_eq!(alpha, 0.5);
        assert_eq!(beta, 0.5);
    }

    #[test]
    fn test_escalated_beta_update() {
        let (alpha, beta) = ContractOutcome::Escalated.to_beta_update();
        assert_eq!(alpha, 0.0);
        assert_eq!(beta, 1.5);
    }

    #[test]
    fn test_rubric_to_feedback_success() {
        let id = Uuid::new_v4();
        let fb = rubric_to_feedback(0.8, "code", id, 100);
        assert!(fb.success);
        assert_eq!(fb.quality_score, Some(0.8));
        assert_eq!(fb.task_category, "code");
    }

    #[test]
    fn test_rubric_to_feedback_failure() {
        let id = Uuid::new_v4();
        let fb = rubric_to_feedback(0.3, "code", id, 50);
        assert!(!fb.success);
        assert_eq!(fb.quality_score, Some(0.3));
    }

    #[test]
    fn test_rubric_to_feedback_boundary() {
        let id = Uuid::new_v4();
        let fb = rubric_to_feedback(0.5, "code", id, 50);
        assert!(fb.success); // 0.5 is the threshold
    }

    #[test]
    fn test_contract_outcome_to_feedback_first_round() {
        let id = Uuid::new_v4();
        let fb = contract_outcome_to_feedback(ContractOutcome::FirstRound, "zone", id, 200);
        assert!(fb.success);
        assert!(fb.quality_score.unwrap() > 0.5);
    }

    #[test]
    fn test_contract_outcome_to_feedback_escalated() {
        let id = Uuid::new_v4();
        let fb = contract_outcome_to_feedback(ContractOutcome::Escalated, "zone", id, 300);
        assert!(!fb.success);
        assert!(fb.quality_score.unwrap() < 0.5);
    }
}
