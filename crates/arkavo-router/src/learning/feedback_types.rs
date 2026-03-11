//! Feedback types for learning credit assignment

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Burst-level (immediate) feedback
///
/// # Format Learning
///
/// To track tool call format success, use category naming convention:
/// - `"format:fence"` - for fence-based tool calls (```tool_name)
/// - `"format:xml"` - for XML-style tool calls (<tool_call>)
/// - `"format:json"` - for raw JSON tool calls
/// - `"format:python"` - for Python function call syntax
///
/// Example:
/// ```ignore
/// let feedback = BurstFeedback::success(burst_id, "format:fence".to_string(), latency_ms);
/// ```
///
/// The existing `category_priors` in `AgentUtility` will automatically track
/// format-specific success rates via Thompson Sampling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurstFeedback {
    /// Burst identifier
    pub burst_id: Uuid,
    /// When this feedback was recorded
    pub timestamp: DateTime<Utc>,
    /// Latency in milliseconds
    pub latency_ms: u64,
    /// Whether the burst succeeded
    pub success: bool,
    /// Optional quality score (0.0 to 1.0)
    pub quality_score: Option<f64>,
    /// Task category for category-specific learning
    /// Use "format:<type>" for format learning (e.g., "format:fence")
    pub task_category: String,
    /// Cost incurred
    pub cost_usd: f64,
    /// Tokens used
    pub tokens_used: u64,
}

impl BurstFeedback {
    /// Create a success feedback
    pub fn success(burst_id: Uuid, task_category: String, latency_ms: u64) -> Self {
        Self {
            burst_id,
            timestamp: Utc::now(),
            latency_ms,
            success: true,
            quality_score: None,
            task_category,
            cost_usd: 0.0,
            tokens_used: 0,
        }
    }

    /// Create a failure feedback
    pub fn failure(burst_id: Uuid, task_category: String, latency_ms: u64) -> Self {
        Self {
            burst_id,
            timestamp: Utc::now(),
            latency_ms,
            success: false,
            quality_score: None,
            task_category,
            cost_usd: 0.0,
            tokens_used: 0,
        }
    }

    /// Set quality score
    pub fn with_quality(mut self, score: f64) -> Self {
        self.quality_score = Some(score.clamp(0.0, 1.0));
        self
    }

    /// Set usage metrics
    pub fn with_usage(mut self, cost_usd: f64, tokens: u64) -> Self {
        self.cost_usd = cost_usd;
        self.tokens_used = tokens;
        self
    }
}

/// Task-level (retrospective) report for credit assignment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalTaskReport {
    /// Task identifier
    pub task_id: Uuid,
    /// When the task completed
    pub completed_at: DateTime<Utc>,
    /// Ordered list of agents that contributed
    pub agent_contributions: Vec<AgentContribution>,
    /// Overall task success (0.0 to 1.0)
    pub final_reward: f64,
    /// Optional quality assessment
    pub quality_metrics: Option<QualityMetrics>,
    /// Optional planner score (when task used architect mode)
    #[serde(default)]
    pub planner_score: Option<super::planner_score::PlannerScore>,
    /// Per-step reward overrides (if non-empty, used instead of uniform discount)
    #[serde(default)]
    pub per_step_rewards: Vec<f64>,
}

impl FinalTaskReport {
    /// Create a successful task report
    pub fn success(task_id: Uuid, contributions: Vec<AgentContribution>) -> Self {
        Self {
            task_id,
            completed_at: Utc::now(),
            agent_contributions: contributions,
            final_reward: 1.0,
            quality_metrics: None,
            planner_score: None,
            per_step_rewards: vec![],
        }
    }

    /// Create a failed task report
    pub fn failure(task_id: Uuid, contributions: Vec<AgentContribution>) -> Self {
        Self {
            task_id,
            completed_at: Utc::now(),
            agent_contributions: contributions,
            final_reward: 0.0,
            quality_metrics: None,
            planner_score: None,
            per_step_rewards: vec![],
        }
    }

    /// Set final reward
    pub fn with_reward(mut self, reward: f64) -> Self {
        self.final_reward = reward.clamp(0.0, 1.0);
        self
    }

    /// Set quality metrics
    pub fn with_quality(mut self, metrics: QualityMetrics) -> Self {
        self.quality_metrics = Some(metrics);
        self
    }
}

/// Contribution from a single agent to a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContribution {
    /// Agent identifier
    pub agent_id: String,
    /// Position in the task sequence (0 = first)
    pub position: usize,
    /// Immediate reward for this step
    pub immediate_reward: f64,
}

/// Quality metrics for a completed task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityMetrics {
    /// Correctness score (0.0 to 1.0)
    pub correctness: f64,
    /// Completeness score (0.0 to 1.0)
    pub completeness: f64,
    /// Efficiency score (0.0 to 1.0)
    pub efficiency: f64,
}

impl QualityMetrics {
    /// Create new quality metrics
    pub fn new(correctness: f64, completeness: f64, efficiency: f64) -> Self {
        Self {
            correctness: correctness.clamp(0.0, 1.0),
            completeness: completeness.clamp(0.0, 1.0),
            efficiency: efficiency.clamp(0.0, 1.0),
        }
    }

    /// Overall quality score (average of components)
    pub fn overall(&self) -> f64 {
        (self.correctness + self.completeness + self.efficiency) / 3.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_burst_feedback_success() {
        let fb = BurstFeedback::success(Uuid::new_v4(), "code".to_string(), 100);
        assert!(fb.success);
        assert!(fb.quality_score.is_none());
        assert_eq!(fb.cost_usd, 0.0);
    }

    #[test]
    fn test_burst_feedback_with_quality() {
        let fb = BurstFeedback::failure(Uuid::new_v4(), "code".to_string(), 50).with_quality(0.8);
        assert!(!fb.success);
        assert_eq!(fb.quality_score, Some(0.8));
    }

    #[test]
    fn test_burst_feedback_quality_clamped() {
        let fb = BurstFeedback::success(Uuid::new_v4(), "code".to_string(), 50).with_quality(1.5);
        assert_eq!(fb.quality_score, Some(1.0));
    }

    #[test]
    fn test_final_task_report_success() {
        let report = FinalTaskReport::success(Uuid::new_v4(), vec![]);
        assert_eq!(report.final_reward, 1.0);
        assert!(report.planner_score.is_none());
        assert!(report.per_step_rewards.is_empty());
    }

    #[test]
    fn test_final_task_report_failure() {
        let report = FinalTaskReport::failure(Uuid::new_v4(), vec![]);
        assert_eq!(report.final_reward, 0.0);
    }

    #[test]
    fn test_quality_metrics() {
        let qm = QualityMetrics::new(0.8, 0.6, 0.9);
        let overall = qm.overall();
        assert!((overall - 0.7667).abs() < 0.01);
    }

    #[test]
    fn test_quality_metrics_clamped() {
        let qm = QualityMetrics::new(1.5, -0.1, 0.5);
        assert_eq!(qm.correctness, 1.0);
        assert_eq!(qm.completeness, 0.0);
        assert_eq!(qm.efficiency, 0.5);
    }
}
