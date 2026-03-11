//! Decision trace for routing decisions
//!
//! Every routing decision carries a full trace of why it was made,
//! enabling downstream learning from both successes and failures.

use crate::classifier::TaskCategory;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Full trace of a routing decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTrace {
    /// Unique trace identifier
    pub trace_id: Uuid,
    /// When this decision was made
    pub timestamp: DateTime<Utc>,
    /// Task category from classification
    pub task_category: TaskCategory,
    /// Classification confidence
    pub classification_confidence: f64,
    /// Models that passed budget/exclusion filtering
    pub feasible_models: Vec<String>,
    /// All Thompson Sampling scores (model_name, score)
    pub thompson_scores: Vec<(String, f64)>,
    /// Which model was selected
    pub selected_model: String,
    /// Why it was selected
    pub selection_reason: SelectionReason,
    /// Budget usage at decision time (0.0-1.0)
    pub budget_usage_pct: f64,
    /// Models excluded by cooldown or other filters
    pub excluded_models: Vec<String>,
}

impl DecisionTrace {
    /// Create a new trace for a Thompson Sampling selection
    #[allow(clippy::too_many_arguments)]
    pub fn thompson(
        task_category: TaskCategory,
        confidence: f64,
        feasible: Vec<String>,
        scores: Vec<(String, f64)>,
        selected: &str,
        rank: usize,
        budget_usage: f64,
        excluded: Vec<String>,
    ) -> Self {
        let score = scores
            .iter()
            .find(|(n, _)| n == selected)
            .map(|(_, s)| *s)
            .unwrap_or(0.0);
        Self {
            trace_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            task_category,
            classification_confidence: confidence,
            feasible_models: feasible,
            thompson_scores: scores,
            selected_model: selected.to_string(),
            selection_reason: SelectionReason::ThompsonSampling { score, rank },
            budget_usage_pct: budget_usage,
            excluded_models: excluded,
        }
    }

    /// Create a trace for a single-feasible-model decision
    pub fn single_feasible(
        task_category: TaskCategory,
        confidence: f64,
        model: &str,
        budget_usage: f64,
        excluded: Vec<String>,
    ) -> Self {
        Self {
            trace_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            task_category,
            classification_confidence: confidence,
            feasible_models: vec![model.to_string()],
            thompson_scores: vec![],
            selected_model: model.to_string(),
            selection_reason: SelectionReason::SingleFeasible,
            budget_usage_pct: budget_usage,
            excluded_models: excluded,
        }
    }

    /// Create a trace for a budget-constrained decision
    pub fn budget_constrained(
        task_category: TaskCategory,
        confidence: f64,
        model: &str,
        budget_usage: f64,
    ) -> Self {
        Self {
            trace_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            task_category,
            classification_confidence: confidence,
            feasible_models: vec![model.to_string()],
            thompson_scores: vec![],
            selected_model: model.to_string(),
            selection_reason: SelectionReason::BudgetConstrained,
            budget_usage_pct: budget_usage,
            excluded_models: vec![],
        }
    }

    /// Create a trace for a fallback/heuristic decision
    pub fn fallback(task_category: TaskCategory, confidence: f64, model: &str) -> Self {
        Self {
            trace_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            task_category,
            classification_confidence: confidence,
            feasible_models: vec![model.to_string()],
            thompson_scores: vec![],
            selected_model: model.to_string(),
            selection_reason: SelectionReason::Fallback,
            budget_usage_pct: 0.0,
            excluded_models: vec![],
        }
    }
}

/// Why a particular model was selected
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SelectionReason {
    /// Selected by Thompson Sampling with given score and rank
    ThompsonSampling { score: f64, rank: usize },
    /// Forced by budget constraints (over threshold)
    BudgetConstrained,
    /// Only one model was feasible
    SingleFeasible,
    /// Probationary exploration
    Probationary,
    /// Heuristic fallback (no learning data)
    Fallback,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thompson_trace() {
        let trace = DecisionTrace::thompson(
            TaskCategory::CodeGeneration,
            0.9,
            vec!["model-a".into(), "model-b".into()],
            vec![("model-a".into(), 0.8), ("model-b".into(), 0.6)],
            "model-a",
            0,
            0.3,
            vec!["model-c".into()],
        );
        assert_eq!(trace.selected_model, "model-a");
        assert_eq!(trace.feasible_models.len(), 2);
        assert_eq!(trace.excluded_models, vec!["model-c"]);
        assert!(matches!(
            trace.selection_reason,
            SelectionReason::ThompsonSampling { score, rank } if (score - 0.8).abs() < 0.01 && rank == 0
        ));
    }

    #[test]
    fn test_single_feasible_trace() {
        let trace = DecisionTrace::single_feasible(
            TaskCategory::General,
            0.7,
            "qwen3.5-0.8b",
            0.95,
            vec!["excluded".into()],
        );
        assert!(matches!(
            trace.selection_reason,
            SelectionReason::SingleFeasible
        ));
        assert_eq!(trace.feasible_models.len(), 1);
    }

    #[test]
    fn test_budget_constrained_trace() {
        let trace =
            DecisionTrace::budget_constrained(TaskCategory::FrontendUI, 0.85, "ministral-3b", 0.92);
        assert!(matches!(
            trace.selection_reason,
            SelectionReason::BudgetConstrained
        ));
        assert!(trace.budget_usage_pct > 0.9);
    }

    #[test]
    fn test_fallback_trace() {
        let trace = DecisionTrace::fallback(TaskCategory::CodeSearch, 0.6, "qwen3.5-0.8b");
        assert!(matches!(trace.selection_reason, SelectionReason::Fallback));
    }

    #[test]
    fn test_trace_serialization_roundtrip() {
        let trace = DecisionTrace::thompson(
            TaskCategory::BackendAPI,
            0.95,
            vec!["model-a".into()],
            vec![("model-a".into(), 0.75)],
            "model-a",
            0,
            0.1,
            vec![],
        );
        let json = serde_json::to_string(&trace).unwrap();
        let deserialized: DecisionTrace = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.trace_id, trace.trace_id);
        assert_eq!(deserialized.selected_model, "model-a");
    }
}
