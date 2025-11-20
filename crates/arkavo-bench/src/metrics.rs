use arkavo_router::{IssueType, JudgmentResult};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchMetrics {
    pub instance_id: String,
    pub resolved: bool,
    pub wall_time_ms: u64,
    pub api_calls: usize,
    pub total_tokens: usize,
    pub estimated_cost_usd: f64,
    pub error_message: Option<String>,
    pub quality_gate_passed: Option<bool>,
    pub quality_retries: u32,
    pub issue_type: Option<String>,
    pub judge_reason: Option<String>,
}

impl BenchMetrics {
    pub fn new(instance_id: String) -> Self {
        Self {
            instance_id,
            resolved: false,
            wall_time_ms: 0,
            api_calls: 0,
            total_tokens: 0,
            estimated_cost_usd: 0.0,
            error_message: None,
            quality_gate_passed: None,
            quality_retries: 0,
            issue_type: None,
            judge_reason: None,
        }
    }

    pub fn set_resolved(&mut self, resolved: bool) {
        self.resolved = resolved;
    }

    pub fn set_wall_time(&mut self, duration: Duration) {
        self.wall_time_ms = duration.as_millis() as u64;
    }

    pub fn add_api_call(&mut self, tokens: usize) {
        self.api_calls += 1;
        self.total_tokens += tokens;
        self.estimated_cost_usd += Self::estimate_cost(tokens);
    }

    pub fn set_error(&mut self, error: String) {
        self.error_message = Some(error);
    }

    pub fn set_judgment(&mut self, judgment: &JudgmentResult, retries: u32) {
        self.quality_gate_passed = Some(judgment.passed);
        self.quality_retries = retries;
        self.issue_type = Some(Self::issue_type_to_string(&judgment.issue_type));
        self.judge_reason = judgment.reason.clone();
    }

    fn issue_type_to_string(issue_type: &IssueType) -> String {
        match issue_type {
            IssueType::HallucinatedTool => "hallucinated_tool".to_string(),
            IssueType::InvalidParams => "invalid_params".to_string(),
            IssueType::Refusal => "refusal".to_string(),
            IssueType::OffTopic => "off_topic".to_string(),
            IssueType::MissingToolUse => "missing_tool_use".to_string(),
            IssueType::None => "none".to_string(),
        }
    }

    fn estimate_cost(tokens: usize) -> f64 {
        let input_cost_per_1k = 0.003;
        let output_cost_per_1k = 0.015;
        let avg_cost_per_1k = (input_cost_per_1k + output_cost_per_1k) / 2.0;
        (tokens as f64 / 1000.0) * avg_cost_per_1k
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSummary {
    pub total_instances: usize,
    pub resolved_count: usize,
    pub resolved_percentage: f64,
    pub avg_wall_time_ms: u64,
    pub total_cost_usd: f64,
    pub total_api_calls: usize,
    pub total_tokens: usize,
    pub quality_gate_pass_rate: f64,
    pub avg_quality_retries: f64,
    pub issue_type_breakdown: std::collections::HashMap<String, usize>,
}

impl BenchSummary {
    pub fn from_metrics(metrics: &[BenchMetrics]) -> Self {
        let total_instances = metrics.len();
        let resolved_count = metrics.iter().filter(|m| m.resolved).count();
        let resolved_percentage = if total_instances > 0 {
            (resolved_count as f64 / total_instances as f64) * 100.0
        } else {
            0.0
        };

        let total_wall_time: u64 = metrics.iter().map(|m| m.wall_time_ms).sum();
        let avg_wall_time_ms = if total_instances > 0 {
            total_wall_time / total_instances as u64
        } else {
            0
        };

        let total_cost_usd: f64 = metrics.iter().map(|m| m.estimated_cost_usd).sum();
        let total_api_calls: usize = metrics.iter().map(|m| m.api_calls).sum();
        let total_tokens: usize = metrics.iter().map(|m| m.total_tokens).sum();

        let quality_gate_passed_count = metrics
            .iter()
            .filter(|m| m.quality_gate_passed.unwrap_or(false))
            .count();
        let quality_gate_pass_rate = if total_instances > 0 {
            (quality_gate_passed_count as f64 / total_instances as f64) * 100.0
        } else {
            0.0
        };

        let total_retries: u32 = metrics.iter().map(|m| m.quality_retries).sum();
        let avg_quality_retries = if total_instances > 0 {
            total_retries as f64 / total_instances as f64
        } else {
            0.0
        };

        let mut issue_type_breakdown = std::collections::HashMap::new();
        for metric in metrics {
            if let Some(issue_type) = &metric.issue_type {
                *issue_type_breakdown.entry(issue_type.clone()).or_insert(0) += 1;
            }
        }

        Self {
            total_instances,
            resolved_count,
            resolved_percentage,
            avg_wall_time_ms,
            total_cost_usd,
            total_api_calls,
            total_tokens,
            quality_gate_pass_rate,
            avg_quality_retries,
            issue_type_breakdown,
        }
    }
}
