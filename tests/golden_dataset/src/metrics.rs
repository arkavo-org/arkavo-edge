//! Metrics collection for golden dataset benchmarks

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Metrics collected during task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMetrics {
    /// Total wall clock time
    pub total_duration_ms: u64,
    /// Time spent in Conductor decomposition
    pub decomposition_duration_ms: u64,
    /// Time spent in agent selection (Router)
    pub selection_duration_ms: u64,
    /// Time spent in execution
    pub execution_duration_ms: u64,
    /// Time spent in Critic verification
    pub verification_duration_ms: u64,
    /// Number of subtasks created
    pub subtask_count: u32,
    /// Number of successful subtasks
    pub successful_subtasks: u32,
    /// Number of failed subtasks
    pub failed_subtasks: u32,
    /// Number of retries
    pub retry_count: u32,
    /// Total tokens consumed
    pub tokens_used: u64,
    /// Estimated cost in USD
    pub cost_usd: f64,
    /// Agent selection decisions
    pub agent_selections: Vec<AgentSelection>,
    /// Critic check results
    pub critic_results: Vec<CriticCheckResult>,
}

impl Default for TaskMetrics {
    fn default() -> Self {
        Self {
            total_duration_ms: 0,
            decomposition_duration_ms: 0,
            selection_duration_ms: 0,
            execution_duration_ms: 0,
            verification_duration_ms: 0,
            subtask_count: 0,
            successful_subtasks: 0,
            failed_subtasks: 0,
            retry_count: 0,
            tokens_used: 0,
            cost_usd: 0.0,
            agent_selections: Vec::new(),
            critic_results: Vec::new(),
        }
    }
}

/// Record of an agent selection decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSelection {
    pub subtask_id: String,
    pub selected_agent: String,
    pub score: f64,
    pub candidates: Vec<AgentCandidate>,
}

/// Candidate agent considered during selection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCandidate {
    pub agent_id: String,
    pub score: f64,
    pub utility_prior: f64,
}

/// Result of a Critic check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticCheckResult {
    pub check_id: String,
    pub passed: bool,
    pub latency_ms: u64,
    pub severity: Option<String>,
    pub message: Option<String>,
}

/// Collector for metrics during test execution
#[derive(Debug)]
pub struct MetricsCollector {
    start_time: Instant,
    metrics: TaskMetrics,
    phase_start: Option<(MetricsPhase, Instant)>,
}

#[derive(Debug, Clone, Copy)]
pub enum MetricsPhase {
    Decomposition,
    Selection,
    Execution,
    Verification,
}

impl MetricsCollector {
    /// Create a new collector
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            metrics: TaskMetrics::default(),
            phase_start: None,
        }
    }

    /// Start timing a phase
    pub fn start_phase(&mut self, phase: MetricsPhase) {
        self.end_current_phase();
        self.phase_start = Some((phase, Instant::now()));
    }

    /// End the current phase and record its duration
    pub fn end_current_phase(&mut self) {
        if let Some((phase, start)) = self.phase_start.take() {
            let duration = start.elapsed().as_millis() as u64;
            match phase {
                MetricsPhase::Decomposition => self.metrics.decomposition_duration_ms = duration,
                MetricsPhase::Selection => self.metrics.selection_duration_ms += duration,
                MetricsPhase::Execution => self.metrics.execution_duration_ms += duration,
                MetricsPhase::Verification => self.metrics.verification_duration_ms += duration,
            }
        }
    }

    /// Record subtask creation
    pub fn record_subtask(&mut self) {
        self.metrics.subtask_count += 1;
    }

    /// Record subtask completion
    pub fn record_subtask_success(&mut self) {
        self.metrics.successful_subtasks += 1;
    }

    /// Record subtask failure
    pub fn record_subtask_failure(&mut self) {
        self.metrics.failed_subtasks += 1;
    }

    /// Record a retry
    pub fn record_retry(&mut self) {
        self.metrics.retry_count += 1;
    }

    /// Record token usage
    pub fn record_tokens(&mut self, tokens: u64) {
        self.metrics.tokens_used += tokens;
    }

    /// Record cost
    pub fn record_cost(&mut self, cost: f64) {
        self.metrics.cost_usd += cost;
    }

    /// Record an agent selection decision
    pub fn record_agent_selection(&mut self, selection: AgentSelection) {
        self.metrics.agent_selections.push(selection);
    }

    /// Record a Critic check result
    pub fn record_critic_check(&mut self, result: CriticCheckResult) {
        self.metrics.critic_results.push(result);
    }

    /// Finalize and return metrics
    pub fn finalize(mut self) -> TaskMetrics {
        self.end_current_phase();
        self.metrics.total_duration_ms = self.start_time.elapsed().as_millis() as u64;
        self.metrics
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Aggregate metrics across multiple tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSummary {
    /// Total tasks run
    pub total_tasks: u32,
    /// Tasks that passed
    pub passed_tasks: u32,
    /// Tasks that failed
    pub failed_tasks: u32,
    /// Overall success rate
    pub success_rate: f64,
    /// Average task duration
    pub avg_duration_ms: f64,
    /// Average tokens per task
    pub avg_tokens: f64,
    /// Average cost per task
    pub avg_cost_usd: f64,
    /// P50 latency
    pub p50_duration_ms: u64,
    /// P95 latency
    pub p95_duration_ms: u64,
    /// P99 latency
    pub p99_duration_ms: u64,
    /// Total cost
    pub total_cost_usd: f64,
    /// Per-category breakdown
    pub category_breakdown: Vec<CategoryMetrics>,
}

/// Metrics breakdown by category
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryMetrics {
    pub category: String,
    pub task_count: u32,
    pub success_rate: f64,
    pub avg_duration_ms: f64,
}

impl BenchmarkSummary {
    /// Compute summary from a list of task results
    pub fn from_results(results: &[crate::GoldenResult]) -> Self {
        if results.is_empty() {
            return Self::empty();
        }

        let total_tasks = results.len() as u32;
        let passed_tasks = results.iter().filter(|r| r.passed).count() as u32;
        let failed_tasks = total_tasks - passed_tasks;
        let success_rate = f64::from(passed_tasks) / f64::from(total_tasks);

        let durations: Vec<u64> = results
            .iter()
            .map(|r| r.metrics.total_duration_ms)
            .collect();
        let avg_duration_ms = durations.iter().sum::<u64>() as f64 / durations.len() as f64;

        let avg_tokens =
            results.iter().map(|r| r.metrics.tokens_used).sum::<u64>() as f64 / total_tasks as f64;
        let total_cost_usd: f64 = results.iter().map(|r| r.metrics.cost_usd).sum();
        let avg_cost_usd = total_cost_usd / f64::from(total_tasks);

        let mut sorted_durations = durations;
        sorted_durations.sort_unstable();

        let p50_duration_ms = percentile(&sorted_durations, 50);
        let p95_duration_ms = percentile(&sorted_durations, 95);
        let p99_duration_ms = percentile(&sorted_durations, 99);

        Self {
            total_tasks,
            passed_tasks,
            failed_tasks,
            success_rate,
            avg_duration_ms,
            avg_tokens,
            avg_cost_usd,
            p50_duration_ms,
            p95_duration_ms,
            p99_duration_ms,
            total_cost_usd,
            category_breakdown: Vec::new(), // TODO: populate from task specs
        }
    }

    fn empty() -> Self {
        Self {
            total_tasks: 0,
            passed_tasks: 0,
            failed_tasks: 0,
            success_rate: 0.0,
            avg_duration_ms: 0.0,
            avg_tokens: 0.0,
            avg_cost_usd: 0.0,
            p50_duration_ms: 0,
            p95_duration_ms: 0,
            p99_duration_ms: 0,
            total_cost_usd: 0.0,
            category_breakdown: Vec::new(),
        }
    }
}

fn percentile(sorted: &[u64], p: u32) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() as f64 * f64::from(p) / 100.0).ceil() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_metrics_collector() {
        let mut collector = MetricsCollector::new();

        collector.start_phase(MetricsPhase::Decomposition);
        std::thread::sleep(Duration::from_millis(1));
        collector.end_current_phase();

        collector.record_subtask();
        collector.record_subtask();
        collector.record_subtask_success();
        collector.record_tokens(1000);
        collector.record_cost(0.01);

        let metrics = collector.finalize();
        assert_eq!(metrics.subtask_count, 2);
        assert_eq!(metrics.successful_subtasks, 1);
        assert_eq!(metrics.tokens_used, 1000);
        assert!(metrics.total_duration_ms > 0);
    }

    #[test]
    fn test_percentile() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        // ceil-based percentile: idx = ceil(10 * 50 / 100) = 5, so data[5] = 6
        assert_eq!(percentile(&data, 50), 6);
        assert_eq!(percentile(&data, 95), 10);
    }

    #[test]
    fn test_benchmark_summary() {
        let results = vec![
            crate::GoldenResult {
                task_id: "TEST-01".to_string(),
                passed: true,
                validation_results: vec![],
                metrics: TaskMetrics {
                    total_duration_ms: 100,
                    tokens_used: 1000,
                    cost_usd: 0.01,
                    ..Default::default()
                },
                error: None,
            },
            crate::GoldenResult {
                task_id: "TEST-02".to_string(),
                passed: false,
                validation_results: vec![],
                metrics: TaskMetrics {
                    total_duration_ms: 200,
                    tokens_used: 2000,
                    cost_usd: 0.02,
                    ..Default::default()
                },
                error: Some("Test failure".to_string()),
            },
        ];

        let summary = BenchmarkSummary::from_results(&results);
        assert_eq!(summary.total_tasks, 2);
        assert_eq!(summary.passed_tasks, 1);
        assert_eq!(summary.success_rate, 0.5);
    }
}
