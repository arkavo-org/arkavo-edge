use arkavo_budget::TokenCost;
use arkavo_router::{OrchestratorMetrics, RoutingMetrics};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROIDashboard {
    pub session_stats: SessionStats,
    pub cost_breakdown: CostBreakdown,
    pub model_distribution: ModelDistribution,
    pub budget_health: BudgetHealth,
    pub recommendations: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_allocations: Vec<arkavo_budget::AgentAllocationSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub total_tasks: u64,
    pub total_cost: f64,
    pub total_savings: f64,
    pub savings_percent: f64,
    pub burn_rate_per_hour: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub by_category: HashMap<String, f64>,
    pub by_model: HashMap<String, f64>,
    pub cloud_vs_local: CloudVsLocal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudVsLocal {
    pub cloud_cost: f64,
    pub local_cost: f64,
    pub cloud_tasks: u64,
    pub local_tasks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDistribution {
    pub local_percent: f64,
    pub cloud_percent: f64,
    pub tasks_by_model: HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetHealth {
    pub remaining_budget: f64,
    pub budget_limit: f64,
    pub usage_percent: f64,
    pub estimated_runway_hours: f64,
    pub tasks_remaining: u32,
    pub status: BudgetHealthStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BudgetHealthStatus {
    Healthy,
    Warning,
    Critical,
    Exhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostMetrics {
    pub time_range: String,
    pub total_cost: f64,
    pub total_savings: f64,
    pub cost_by_hour: Vec<HourlyCost>,
    pub average_task_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HourlyCost {
    pub hour: String,
    pub cost: f64,
    pub tasks: u64,
}

pub struct ROICalculator;

impl ROICalculator {
    pub fn calculate_dashboard(
        routing_metrics: &RoutingMetrics,
        orchestrator_metrics: &OrchestratorMetrics,
        session_limit: Option<TokenCost>,
        session_spent: TokenCost,
    ) -> ROIDashboard {
        Self::calculate_dashboard_with_allocations(
            routing_metrics,
            orchestrator_metrics,
            session_limit,
            session_spent,
            vec![],
        )
    }

    pub fn calculate_dashboard_with_allocations(
        routing_metrics: &RoutingMetrics,
        orchestrator_metrics: &OrchestratorMetrics,
        session_limit: Option<TokenCost>,
        session_spent: TokenCost,
        agent_allocations: Vec<arkavo_budget::AgentAllocationSummary>,
    ) -> ROIDashboard {
        let session_stats = Self::calculate_session_stats(routing_metrics, orchestrator_metrics);

        let cost_breakdown = Self::calculate_cost_breakdown(routing_metrics);

        let model_distribution = Self::calculate_model_distribution(routing_metrics);

        let budget_health =
            Self::calculate_budget_health(routing_metrics, session_limit, session_spent);

        let recommendations =
            Self::generate_recommendations(routing_metrics, &budget_health, orchestrator_metrics);

        ROIDashboard {
            session_stats,
            cost_breakdown,
            model_distribution,
            budget_health,
            recommendations,
            agent_allocations,
        }
    }

    fn calculate_session_stats(
        routing_metrics: &RoutingMetrics,
        orchestrator_metrics: &OrchestratorMetrics,
    ) -> SessionStats {
        let total_tasks = routing_metrics.total_routes;
        let total_cost = routing_metrics.total_estimated_cost;
        let total_savings = routing_metrics.total_cost_saved;
        let savings_percent = routing_metrics.cost_savings_percent();

        let time_elapsed = routing_metrics
            .last_updated
            .signed_duration_since(routing_metrics.started_at)
            .num_seconds() as f64
            / 3600.0;

        let burn_rate_per_hour = if time_elapsed > 0.0 {
            total_cost / time_elapsed
        } else {
            0.0
        };

        SessionStats {
            total_tasks,
            total_cost,
            total_savings: total_savings + orchestrator_metrics.total_budget_saved,
            savings_percent,
            burn_rate_per_hour,
        }
    }

    fn calculate_cost_breakdown(routing_metrics: &RoutingMetrics) -> CostBreakdown {
        let mut by_category = HashMap::new();
        let mut by_model = HashMap::new();

        for (category, count) in &routing_metrics.routes_by_category {
            let avg_cost = routing_metrics.average_cost();
            by_category.insert(category.clone(), avg_cost * (*count as f64));
        }

        for (model, count) in &routing_metrics.routes_by_model {
            let avg_cost = routing_metrics.average_cost();
            by_model.insert(model.clone(), avg_cost * (*count as f64));
        }

        let local_tasks: u64 = routing_metrics
            .routes_by_model
            .iter()
            .filter(|(name, _)| name.contains("gemma"))
            .map(|(_, count)| count)
            .sum();

        let cloud_tasks = routing_metrics.total_routes - local_tasks;

        let cloud_vs_local = CloudVsLocal {
            cloud_cost: routing_metrics.total_estimated_cost,
            local_cost: 0.0,
            cloud_tasks,
            local_tasks,
        };

        CostBreakdown {
            by_category,
            by_model,
            cloud_vs_local,
        }
    }

    fn calculate_model_distribution(routing_metrics: &RoutingMetrics) -> ModelDistribution {
        let local_percent = routing_metrics.local_model_usage_percent();
        let cloud_percent = 100.0 - local_percent;

        ModelDistribution {
            local_percent,
            cloud_percent,
            tasks_by_model: routing_metrics.routes_by_model.clone(),
        }
    }

    fn calculate_budget_health(
        routing_metrics: &RoutingMetrics,
        session_limit: Option<TokenCost>,
        session_spent: TokenCost,
    ) -> BudgetHealth {
        let session_limit_dollars = session_limit.map(|l| l.as_dollars()).unwrap_or(f64::MAX);
        let session_spent_dollars = session_spent.as_dollars();

        let remaining_budget = if session_limit_dollars < f64::MAX {
            session_limit_dollars - session_spent_dollars
        } else {
            f64::MAX
        };

        let usage_percent = if session_limit_dollars > 0.0 {
            (session_spent_dollars / session_limit_dollars) * 100.0
        } else {
            0.0
        };

        let burn_rate = routing_metrics.average_cost();
        let tasks_remaining = if burn_rate > 0.0 && remaining_budget < f64::MAX {
            (remaining_budget / burn_rate) as u32
        } else {
            u32::MAX
        };

        let estimated_runway_hours = if burn_rate > 0.0 && remaining_budget < f64::MAX {
            remaining_budget / (burn_rate * 10.0)
        } else {
            f64::MAX
        };

        let status = if usage_percent >= 100.0 {
            BudgetHealthStatus::Exhausted
        } else if usage_percent >= 90.0 {
            BudgetHealthStatus::Critical
        } else if usage_percent >= 75.0 {
            BudgetHealthStatus::Warning
        } else {
            BudgetHealthStatus::Healthy
        };

        BudgetHealth {
            remaining_budget,
            budget_limit: session_limit_dollars,
            usage_percent,
            estimated_runway_hours,
            tasks_remaining,
            status,
        }
    }

    fn generate_recommendations(
        routing_metrics: &RoutingMetrics,
        budget_health: &BudgetHealth,
        orchestrator_metrics: &OrchestratorMetrics,
    ) -> Vec<String> {
        let mut recommendations = Vec::new();

        match budget_health.status {
            BudgetHealthStatus::Exhausted => {
                recommendations.push(
                    "⛔ Budget exhausted. Switch to 100% local models immediately.".to_string(),
                );
            }
            BudgetHealthStatus::Critical => {
                recommendations.push(format!(
                    "🚨 CRITICAL: Only {:.1}% budget remaining. Enable aggressive local routing.",
                    100.0 - budget_health.usage_percent
                ));
            }
            BudgetHealthStatus::Warning => {
                recommendations.push(format!(
                    "⚠️ Budget at {:.1}%. Consider increasing local model usage.",
                    budget_health.usage_percent
                ));
            }
            BudgetHealthStatus::Healthy => {
                if routing_metrics.local_model_usage_percent() < 30.0 {
                    recommendations.push(
                        "💡 Budget healthy. Could increase cost savings with more local routing."
                            .to_string(),
                    );
                }
            }
        }

        if orchestrator_metrics.budget_switches > 0 {
            recommendations.push(format!(
                "✅ Auto-switched to local models {} times to stay within budget.",
                orchestrator_metrics.budget_switches
            ));
        }

        if routing_metrics.cost_savings_percent() > 60.0 {
            recommendations.push(format!(
                "🎉 Excellent! {:.1}% cost savings vs cloud-only.",
                routing_metrics.cost_savings_percent()
            ));
        }

        recommendations
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_roi_dashboard_calculation() {
        let mut routing_metrics = RoutingMetrics::new();
        routing_metrics.total_routes = 100;
        routing_metrics.total_estimated_cost = 0.50;
        routing_metrics.total_cost_saved = 1.20;

        let orch_metrics = OrchestratorMetrics::new();

        let dashboard = ROICalculator::calculate_dashboard(
            &routing_metrics,
            &orch_metrics,
            Some(TokenCost::from_dollars(10.0)),
            TokenCost::from_dollars(2.0),
        );

        assert_eq!(dashboard.session_stats.total_tasks, 100);
        assert!(dashboard.session_stats.total_savings > 0.0);
        assert!(!dashboard.recommendations.is_empty());
    }

    #[test]
    fn test_budget_health_status() {
        let routing_metrics = RoutingMetrics::new();
        let orch_metrics = OrchestratorMetrics::new();

        let dashboard_healthy = ROICalculator::calculate_dashboard(
            &routing_metrics,
            &orch_metrics,
            Some(TokenCost::from_dollars(100.0)),
            TokenCost::from_dollars(10.0),
        );
        assert_eq!(
            dashboard_healthy.budget_health.status,
            BudgetHealthStatus::Healthy
        );

        let dashboard_critical = ROICalculator::calculate_dashboard(
            &routing_metrics,
            &orch_metrics,
            Some(TokenCost::from_dollars(100.0)),
            TokenCost::from_dollars(95.0),
        );
        assert_eq!(
            dashboard_critical.budget_health.status,
            BudgetHealthStatus::Critical
        );
    }
}
