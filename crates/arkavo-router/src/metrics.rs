use crate::classifier::{Classification, TaskCategory};
use crate::decision::RoutingDecision;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingMetrics {
    pub total_routes: u64,
    pub routes_by_category: HashMap<String, u64>,
    pub routes_by_model: HashMap<String, u64>,
    pub total_cost_saved: f64,
    pub total_estimated_cost: f64,
    pub average_confidence: f64,
    pub started_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
}

impl RoutingMetrics {
    pub fn new() -> Self {
        Self {
            total_routes: 0,
            routes_by_category: HashMap::new(),
            routes_by_model: HashMap::new(),
            total_cost_saved: 0.0,
            total_estimated_cost: 0.0,
            average_confidence: 0.0,
            started_at: Utc::now(),
            last_updated: Utc::now(),
        }
    }

    pub fn record_routing(&mut self, classification: &Classification, decision: &RoutingDecision) {
        self.total_routes += 1;

        *self
            .routes_by_category
            .entry(classification.category.as_str().to_string())
            .or_insert(0) += 1;

        *self
            .routes_by_model
            .entry(decision.recommended_model.name().to_string())
            .or_insert(0) += 1;

        self.total_estimated_cost += decision.estimated_cost_usd;

        let cost_saved = self.calculate_cost_saved(classification.category, decision);
        self.total_cost_saved += cost_saved;

        self.average_confidence = self.average_confidence.mul_add(
            (self.total_routes - 1) as f64,
            classification.confidence as f64,
        ) / self.total_routes as f64;

        self.last_updated = Utc::now();
    }

    fn calculate_cost_saved(&self, category: TaskCategory, decision: &RoutingDecision) -> f64 {
        let baseline_cost = self.baseline_cost(category);

        if decision.recommended_model.is_local() {
            baseline_cost - decision.estimated_cost_usd
        } else {
            0.0
        }
    }

    fn baseline_cost(&self, category: TaskCategory) -> f64 {
        let tokens = category.estimated_tokens();

        let input_cost = (tokens.input as f64 / 1_000_000.0) * 1.25;
        let output_cost = (tokens.output as f64 / 1_000_000.0) * 5.00;

        input_cost + output_cost
    }

    pub fn cost_savings_percent(&self) -> f64 {
        let total_baseline = self.total_estimated_cost + self.total_cost_saved;
        if total_baseline == 0.0 {
            return 0.0;
        }

        (self.total_cost_saved / total_baseline) * 100.0
    }

    pub fn local_model_usage_percent(&self) -> f64 {
        if self.total_routes == 0 {
            return 0.0;
        }

        let local_routes: u64 = self
            .routes_by_model
            .iter()
            .filter(|(name, _)| name.contains("gemma"))
            .map(|(_, count)| count)
            .sum();

        (local_routes as f64 / self.total_routes as f64) * 100.0
    }

    pub fn average_cost(&self) -> f64 {
        if self.total_routes == 0 {
            0.0
        } else {
            self.total_estimated_cost / self.total_routes as f64
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "Routing Metrics:\n\
             - Total routes: {}\n\
             - Average confidence: {:.2}%\n\
             - Cost savings: ${:.4} ({:.1}%)\n\
             - Local model usage: {:.1}%\n\
             - Categories: {:?}\n\
             - Models: {:?}",
            self.total_routes,
            self.average_confidence * 100.0,
            self.total_cost_saved,
            self.cost_savings_percent(),
            self.local_model_usage_percent(),
            self.routes_by_category,
            self.routes_by_model
        )
    }
}

impl Default for RoutingMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::decision::ModelChoice;

    #[test]
    fn test_metrics_recording() {
        let mut metrics = RoutingMetrics::new();

        let classification = Classification {
            category: TaskCategory::CodeSearch,
            confidence: 0.85,
            reasoning: "Test".to_string(),
        };

        let decision = RoutingDecision::new(
            ModelChoice::LocalGemma4B,
            TaskCategory::CodeSearch,
            0.85,
            "Test routing".to_string(),
        );

        metrics.record_routing(&classification, &decision);

        assert_eq!(metrics.total_routes, 1);
        assert_eq!(*metrics.routes_by_category.get("code_search").unwrap(), 1);
        assert_eq!(*metrics.routes_by_model.get("gemma-3-4b-it").unwrap(), 1);
    }

    #[test]
    fn test_cost_savings() {
        let mut metrics = RoutingMetrics::new();

        let classification = Classification {
            category: TaskCategory::CodeSearch,
            confidence: 0.90,
            reasoning: "Test".to_string(),
        };

        let decision = RoutingDecision::new(
            ModelChoice::LocalGemma4B,
            TaskCategory::CodeSearch,
            0.90,
            "Local model".to_string(),
        );

        metrics.record_routing(&classification, &decision);

        assert!(metrics.total_cost_saved > 0.0);
        assert!(metrics.cost_savings_percent() > 0.0);
    }

    #[test]
    fn test_local_model_usage() {
        let mut metrics = RoutingMetrics::new();

        let local_decision = RoutingDecision::new(
            ModelChoice::LocalGemma4B,
            TaskCategory::CodeSearch,
            0.90,
            "Local".to_string(),
        );

        let cloud_decision = RoutingDecision::new(
            ModelChoice::GeminiFlash,
            TaskCategory::FrontendUI,
            0.90,
            "Cloud".to_string(),
        );

        let classification = Classification {
            category: TaskCategory::CodeSearch,
            confidence: 0.90,
            reasoning: "Test".to_string(),
        };

        metrics.record_routing(&classification, &local_decision);
        metrics.record_routing(&classification, &cloud_decision);

        assert_eq!(metrics.local_model_usage_percent(), 50.0);
    }
}
