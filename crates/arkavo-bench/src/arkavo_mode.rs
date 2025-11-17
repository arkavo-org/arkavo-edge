use crate::metrics::BenchMetrics;
use crate::solution_applier::SolutionApplier;
use crate::{BenchError, Result, SweBenchInstance};
use arkavo_context::{ProblemStatement, PromptTemplate};
use arkavo_orchestrator::{CodeSolver, SolverConfig};
use arkavo_router::Router;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

pub struct ArkavoMode {
    solver: CodeSolver,
    applier: SolutionApplier,
}

impl ArkavoMode {
    pub async fn new(router: Arc<Router>) -> Result<Self> {
        let config = SolverConfig {
            max_context_tokens: 8000,
            max_quality_retries: 3,
            include_dependencies: true,
            include_tests: true,
            search_depth: 5,
        };

        let solver = CodeSolver::new(router, Some(config))
            .await
            .map_err(|e| BenchError::ExecutionFailed(format!("Failed to create solver: {}", e)))?;

        let applier = SolutionApplier::new(300);

        Ok(Self { solver, applier })
    }

    pub async fn run_instance(
        &self,
        instance: &SweBenchInstance,
        workspace_path: &Path,
    ) -> Result<BenchMetrics> {
        let mut metrics = BenchMetrics::new(instance.instance_id.clone());
        let start = Instant::now();

        info!(
            "Running arkavo-assisted mode for instance: {}",
            instance.instance_id
        );

        let problem = self.build_problem_statement(instance);

        let solver_result = match self
            .solver
            .solve(workspace_path, &problem, PromptTemplate::CodeFix)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                metrics.set_error(format!("Solver failed: {}", e));
                metrics.set_wall_time(start.elapsed());
                return Ok(metrics);
            }
        };

        debug!(
            "Solution generated in {}ms, {} tokens",
            solver_result.metrics.solution_gen_time_ms, solver_result.metrics.total_tokens
        );

        metrics.set_judgment(
            &solver_result.judgment,
            solver_result.metrics.quality_retries as u32,
        );
        metrics.add_api_call(solver_result.metrics.total_tokens as usize);

        if !solver_result.judgment.passed {
            warn!(
                "Quality gate failed: {:?}",
                solver_result.judgment.issue_type
            );
            metrics.set_error(format!(
                "Quality gate failed: {}",
                solver_result.judgment.reason.unwrap_or_default()
            ));
            metrics.set_wall_time(start.elapsed());
            return Ok(metrics);
        }

        let apply_result = self
            .applier
            .apply_and_test(workspace_path, &solver_result.solution, Some("pytest -xvs"))
            .await?;

        if !apply_result.applied {
            metrics.set_error(
                apply_result
                    .error
                    .unwrap_or_else(|| "Unknown error".to_string()),
            );
            metrics.set_wall_time(start.elapsed());
            return Ok(metrics);
        }

        if let Some(test_result) = apply_result.test_result {
            metrics.set_resolved(test_result.passed);

            if !test_result.passed
                && let Some(error_msg) = test_result.error_message {
                    metrics.set_error(error_msg);
                }

            debug!(
                "Tests completed: {} passed, {} failed",
                test_result.passed_tests, test_result.failed_tests
            );
        }

        metrics.set_wall_time(start.elapsed());
        Ok(metrics)
    }

    fn build_problem_statement(&self, instance: &SweBenchInstance) -> ProblemStatement {
        let hints = instance
            .hints_text
            .as_ref()
            .map(|h| vec![h.clone()])
            .unwrap_or_default();

        ProblemStatement {
            title: format!("Fix issue in {}", instance.repo),
            description: instance.problem_statement.clone(),
            hints,
            repo_name: instance.repo.clone(),
            base_commit: instance.base_commit.clone(),
        }
    }
}

pub struct ComparativeRunner {
    arkavo_mode: ArkavoMode,
}

impl ComparativeRunner {
    pub async fn new(router: Arc<Router>) -> Result<Self> {
        let arkavo_mode = ArkavoMode::new(router).await?;
        Ok(Self { arkavo_mode })
    }

    pub async fn run_comparison(
        &self,
        instance: &SweBenchInstance,
        workspace_path: &Path,
    ) -> Result<ComparisonResult> {
        info!("Running comparative benchmark for {}", instance.instance_id);

        let raw_metrics = self.run_raw_mode(instance, workspace_path).await?;

        let arkavo_metrics = self
            .arkavo_mode
            .run_instance(instance, workspace_path)
            .await?;

        Ok(ComparisonResult {
            instance_id: instance.instance_id.clone(),
            raw_metrics,
            arkavo_metrics,
        })
    }

    async fn run_raw_mode(
        &self,
        instance: &SweBenchInstance,
        _workspace_path: &Path,
    ) -> Result<BenchMetrics> {
        let mut metrics = BenchMetrics::new(format!("{}-raw", instance.instance_id));
        metrics.set_resolved(false);
        Ok(metrics)
    }
}

#[derive(Debug, Clone)]
pub struct ComparisonResult {
    pub instance_id: String,
    pub raw_metrics: BenchMetrics,
    pub arkavo_metrics: BenchMetrics,
}

impl ComparisonResult {
    pub fn improvement_percentage(&self) -> f64 {
        let raw_success = if self.raw_metrics.resolved { 1.0 } else { 0.0 };
        let arkavo_success = if self.arkavo_metrics.resolved {
            1.0
        } else {
            0.0
        };

        if raw_success == 0.0 && arkavo_success == 0.0 {
            0.0
        } else if raw_success == 0.0 {
            100.0
        } else {
            ((arkavo_success - raw_success) / raw_success) * 100.0
        }
    }

    pub fn speedup_factor(&self) -> f64 {
        if self.arkavo_metrics.wall_time_ms == 0 {
            return 0.0;
        }

        self.raw_metrics.wall_time_ms as f64 / self.arkavo_metrics.wall_time_ms as f64
    }

    pub fn cost_difference_usd(&self) -> f64 {
        self.arkavo_metrics.estimated_cost_usd - self.raw_metrics.estimated_cost_usd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comparison_result_improvement() {
        let raw_metrics = BenchMetrics::new("test-1-raw".to_string());
        let mut arkavo_metrics = BenchMetrics::new("test-1-arkavo".to_string());
        arkavo_metrics.set_resolved(true);

        let result = ComparisonResult {
            instance_id: "test-1".to_string(),
            raw_metrics,
            arkavo_metrics,
        };

        assert_eq!(result.improvement_percentage(), 100.0);
    }
}
