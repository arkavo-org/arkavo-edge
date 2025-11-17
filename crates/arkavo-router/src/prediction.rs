use crate::classifier::{Classification, TaskCategory};
use crate::decision::TokenEstimate;
use crate::metrics::RoutingMetrics;
use arkavo_budget::tracker::BudgetStatus;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCostPrediction {
    pub total_estimated_cost: f64,
    pub cost_range: (f64, f64),
    pub estimated_time: Duration,
    pub budget_impact_percent: f64,
    pub confidence: f64,
    pub recommendations: Vec<String>,
    pub tasks_breakdown: Vec<TaskCostEstimate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCostEstimate {
    pub task_description: String,
    pub category: TaskCategory,
    pub estimated_cost: f64,
    pub estimated_time: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetRunway {
    pub remaining_budget: f64,
    pub current_burn_rate: f64,
    pub estimated_runway: Duration,
    pub tasks_remaining: u32,
    pub recommendation: String,
}

pub struct WorkflowCostPredictor {
    metrics_history: Vec<RoutingMetrics>,
    budget_status: Option<BudgetStatus>,
}

impl WorkflowCostPredictor {
    pub fn new() -> Self {
        Self {
            metrics_history: Vec::new(),
            budget_status: None,
        }
    }

    pub fn with_history(mut self, metrics: Vec<RoutingMetrics>) -> Self {
        self.metrics_history = metrics;
        self
    }

    pub fn with_budget_status(mut self, status: BudgetStatus) -> Self {
        self.budget_status = Some(status);
        self
    }

    pub fn predict(&self, tasks: &[Classification]) -> WorkflowCostPrediction {
        let mut total_cost = 0.0;
        let mut total_time = Duration::from_secs(0);
        let mut tasks_breakdown = Vec::new();

        for classification in tasks {
            let task_estimate = self.estimate_task_cost(classification);
            total_cost += task_estimate.estimated_cost;
            total_time += task_estimate.estimated_time;
            tasks_breakdown.push(task_estimate);
        }

        let variance_factor = 0.15;
        let cost_min = total_cost * (1.0 - variance_factor);
        let cost_max = total_cost * (1.0 + variance_factor);

        let budget_impact = if let Some(status) = &self.budget_status {
            (total_cost / (status.session_spent.as_dollars() + total_cost)) * 100.0
        } else {
            0.0
        };

        let confidence = if self.metrics_history.is_empty() {
            0.70
        } else {
            0.85
        };

        let recommendations = self.generate_recommendations(total_cost, &tasks_breakdown);

        WorkflowCostPrediction {
            total_estimated_cost: total_cost,
            cost_range: (cost_min, cost_max),
            estimated_time: total_time,
            budget_impact_percent: budget_impact,
            confidence,
            recommendations,
            tasks_breakdown,
        }
    }

    pub fn calculate_budget_runway(&self, burn_rate: f64) -> Option<BudgetRunway> {
        let status = self.budget_status.as_ref()?;

        let remaining = status.session_spent.as_dollars();

        let tasks_per_hour = if burn_rate > 0.0 {
            (60.0 / burn_rate) as u32
        } else {
            0
        };

        let hours_remaining = if burn_rate > 0.0 {
            remaining / burn_rate
        } else {
            f64::MAX
        };

        let runway = Duration::from_secs((hours_remaining * 3600.0) as u64);

        let recommendation = if hours_remaining < 2.0 {
            "URGENT: Less than 2 hours of budget remaining. Switch to local models immediately."
                .to_string()
        } else if hours_remaining < 8.0 {
            format!(
                "WARNING: {hours_remaining:.1} hours remaining. Consider reducing cloud model usage."
            )
        } else {
            format!("Budget healthy with {hours_remaining:.1} hours remaining at current rate.")
        };

        Some(BudgetRunway {
            remaining_budget: remaining,
            current_burn_rate: burn_rate,
            estimated_runway: runway,
            tasks_remaining: (tasks_per_hour as f64 * hours_remaining) as u32,
            recommendation,
        })
    }

    pub fn estimate_burn_rate(&self) -> f64 {
        if self.metrics_history.is_empty() {
            return 0.0;
        }

        let latest = &self.metrics_history[self.metrics_history.len() - 1];

        if latest.total_routes == 0 {
            return 0.0;
        }

        let time_elapsed = latest
            .last_updated
            .signed_duration_since(latest.started_at)
            .num_seconds() as f64
            / 3600.0;

        if time_elapsed > 0.0 {
            latest.total_estimated_cost / time_elapsed
        } else {
            0.0
        }
    }

    fn estimate_task_cost(&self, classification: &Classification) -> TaskCostEstimate {
        let tokens = classification.category.estimated_tokens();

        let base_cost = self.calculate_base_cost(&tokens);

        let adjusted_cost = if classification.confidence > 0.80 {
            base_cost
        } else {
            base_cost * 1.2
        };

        let estimated_time = self.estimate_task_time(classification.category);

        TaskCostEstimate {
            task_description: format!("{} task", classification.category.as_str()),
            category: classification.category,
            estimated_cost: adjusted_cost,
            estimated_time,
        }
    }

    fn calculate_base_cost(&self, tokens: &TokenEstimate) -> f64 {
        let input_cost = (tokens.input as f64 / 1_000_000.0) * 0.30;
        let output_cost = (tokens.output as f64 / 1_000_000.0) * 2.50;
        input_cost + output_cost
    }

    fn estimate_task_time(&self, category: TaskCategory) -> Duration {
        match category {
            TaskCategory::FrontendUI => Duration::from_secs(3),
            TaskCategory::BackendAPI => Duration::from_secs(10),
            TaskCategory::CodeSearch => Duration::from_secs(1),
            TaskCategory::SecurityScan => Duration::from_secs(2),
            TaskCategory::TestGeneration => Duration::from_secs(12),
            TaskCategory::Documentation => Duration::from_secs(2),
            TaskCategory::Refactoring => Duration::from_secs(5),
            TaskCategory::CodeGeneration => Duration::from_secs(8),
            TaskCategory::VisionAnalysis => Duration::from_secs(4),
            TaskCategory::General => Duration::from_secs(3),
        }
    }

    fn generate_recommendations(&self, total_cost: f64, tasks: &[TaskCostEstimate]) -> Vec<String> {
        let mut recommendations = Vec::new();

        if total_cost > 0.10 {
            recommendations.push(
                "High workflow cost detected. Consider using local models for code search and security tasks.".to_string()
            );
        }

        let code_search_count = tasks
            .iter()
            .filter(|t| matches!(t.category, TaskCategory::CodeSearch))
            .count();

        if code_search_count > 2 {
            recommendations.push(format!(
                "{code_search_count} code search tasks detected. Use local Gemma 4B (free) instead of cloud models."
            ));
        }

        let vision_count = tasks
            .iter()
            .filter(|t| matches!(t.category, TaskCategory::VisionAnalysis))
            .count();

        if vision_count > 0 {
            recommendations.push(format!(
                "{} vision tasks at ${:.4} each. Consider local Gemma 3 vision models when available.",
                vision_count,
                tasks.iter()
                    .filter(|t| matches!(t.category, TaskCategory::VisionAnalysis))
                    .map(|t| t.estimated_cost)
                    .sum::<f64>() / vision_count as f64
            ));
        }

        if tasks.len() > 10 {
            recommendations.push(
                "Large workflow detected. Enable context compression to reduce token costs by 60%."
                    .to_string(),
            );
        }

        recommendations
    }
}

impl Default for WorkflowCostPredictor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_prediction() {
        let predictor = WorkflowCostPredictor::new();

        let tasks = vec![
            Classification {
                category: TaskCategory::FrontendUI,
                confidence: 0.90,
                reasoning: "Test".to_string(),
            },
            Classification {
                category: TaskCategory::CodeSearch,
                confidence: 0.85,
                reasoning: "Test".to_string(),
            },
        ];

        let prediction = predictor.predict(&tasks);

        assert!(prediction.total_estimated_cost > 0.0);
        assert_eq!(prediction.tasks_breakdown.len(), 2);
        assert!(prediction.confidence > 0.0);
    }

    #[test]
    fn test_cost_range_variance() {
        let predictor = WorkflowCostPredictor::new();

        let tasks = vec![Classification {
            category: TaskCategory::BackendAPI,
            confidence: 0.80,
            reasoning: "Test".to_string(),
        }];

        let prediction = predictor.predict(&tasks);

        let (min, max) = prediction.cost_range;
        assert!(min < prediction.total_estimated_cost);
        assert!(max > prediction.total_estimated_cost);
        assert!((max - min) / prediction.total_estimated_cost > 0.20);
    }

    #[test]
    fn test_recommendations_generation() {
        let predictor = WorkflowCostPredictor::new();

        let mut tasks = vec![];
        for _ in 0..5 {
            tasks.push(Classification {
                category: TaskCategory::CodeSearch,
                confidence: 0.90,
                reasoning: "Test".to_string(),
            });
        }

        let prediction = predictor.predict(&tasks);

        assert!(!prediction.recommendations.is_empty());
        assert!(
            prediction
                .recommendations
                .iter()
                .any(|r| r.contains("code search"))
        );
    }

    #[test]
    fn test_budget_runway_calculation() {
        let predictor = WorkflowCostPredictor::new().with_budget_status(BudgetStatus {
            session_spent: arkavo_budget::TokenCost::from_dollars(5.0),
            hourly_spent: arkavo_budget::TokenCost::ZERO,
            daily_spent: arkavo_budget::TokenCost::ZERO,
            monthly_spent: arkavo_budget::TokenCost::ZERO,
            total_spent: arkavo_budget::TokenCost::ZERO,
            last_updated: chrono::Utc::now(),
        });

        let burn_rate = 0.50;
        let runway = predictor.calculate_budget_runway(burn_rate);

        assert!(runway.is_some());
        let runway = runway.unwrap();
        assert_eq!(runway.remaining_budget, 5.0);
        assert_eq!(runway.current_burn_rate, 0.50);
        assert!(runway.tasks_remaining > 0);
    }

    #[test]
    fn test_burn_rate_estimation() {
        let predictor = WorkflowCostPredictor::new();

        let burn_rate = predictor.estimate_burn_rate();
        assert_eq!(burn_rate, 0.0);
    }
}
