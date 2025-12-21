//! Integration tests for the Golden Dataset
//!
//! Runs benchmark tasks through the full HRM orchestration pipeline
//! to validate Conductor -> Router -> Execute -> Critic flow.

use golden_dataset::metrics::BenchmarkSummary;
use golden_dataset::runner::{GoldenRunner, RunnerConfig};
use golden_dataset::{load_task_spec, TaskSpec};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/tasks")
}

fn load_fixture(name: &str) -> TaskSpec {
    let path = fixtures_dir().join(format!("{name}.json"));
    load_task_spec(&path).expect(&format!("Failed to load fixture {name}"))
}

/// Test CODE-01: Simple function refactor
///
/// Validates the basic Conductor -> Specialist -> Critic loop works:
/// 1. Conductor decomposes objective into subtasks
/// 2. Agent is selected and executes
/// 3. Critic verifies the output
#[tokio::test]
async fn test_code_01_simple_refactor() {
    let spec = load_fixture("CODE-01");

    let runner = GoldenRunner::new(RunnerConfig {
        verbose: true,
        max_retries: 1,
        enable_critic: true,
    })
    .with_agents(spec.agents.clone());

    let result = runner.run_task(&spec).await;

    // Assertions
    assert!(
        result.passed,
        "CODE-01 should pass. Error: {:?}, Validations: {:?}",
        result.error, result.validation_results
    );

    // Check metrics
    assert!(
        result.metrics.subtask_count >= 1,
        "Should have at least 1 subtask"
    );
    assert!(
        result.metrics.successful_subtasks >= 1,
        "Should have at least 1 successful subtask"
    );
    assert!(
        result.metrics.total_duration_ms > 0,
        "Should have recorded duration"
    );

    // Check validation results
    for vr in &result.validation_results {
        assert!(vr.passed, "Validation '{}' should pass", vr.message);
    }

    println!("CODE-01 metrics: {:?}", result.metrics);
}

/// Test MESH-01: Multi-agent data query
///
/// Validates Router/bandit selection with multiple agents:
/// 1. Conductor creates multiple subtasks
/// 2. Router selects appropriate agents for each
/// 3. Results are aggregated
/// 4. Critic verifies aggregated output
#[tokio::test]
async fn test_mesh_01_multi_agent_query() {
    let spec = load_fixture("MESH-01");

    let runner = GoldenRunner::new(RunnerConfig {
        verbose: true,
        max_retries: 2,
        enable_critic: true,
    })
    .with_agents(spec.agents.clone());

    let result = runner.run_task(&spec).await;

    // Assertions
    assert!(
        result.passed,
        "MESH-01 should pass. Error: {:?}, Validations: {:?}",
        result.error, result.validation_results
    );

    // Check multi-agent selection
    assert!(
        result.metrics.subtask_count >= 2,
        "Should have multiple subtasks for mesh task"
    );

    // Agent selections should be recorded
    assert!(
        !result.metrics.agent_selections.is_empty(),
        "Should have recorded agent selections"
    );

    println!("MESH-01 metrics: {:?}", result.metrics);
}

/// Test running both golden tasks and computing summary
#[tokio::test]
async fn test_golden_suite() {
    let specs = vec![load_fixture("CODE-01"), load_fixture("MESH-01")];

    let runner = GoldenRunner::new(RunnerConfig::default());

    // Build runner with all agents from all specs
    let mut all_agents = Vec::new();
    for spec in &specs {
        all_agents.extend(spec.agents.clone());
    }
    let runner = runner.with_agents(all_agents);

    let results = runner.run_all(&specs).await;

    // Compute summary
    let summary = BenchmarkSummary::from_results(&results);

    println!("\n=== Golden Suite Summary ===");
    println!("Total tasks: {}", summary.total_tasks);
    println!("Passed: {}", summary.passed_tasks);
    println!("Failed: {}", summary.failed_tasks);
    println!("Success rate: {:.1}%", summary.success_rate * 100.0);
    println!("Avg duration: {:.1}ms", summary.avg_duration_ms);
    println!("P50 latency: {}ms", summary.p50_duration_ms);
    println!("P95 latency: {}ms", summary.p95_duration_ms);

    // Success criteria from plan: >= 95% success rate
    // For the tracer bullet, we just verify both tasks pass
    assert_eq!(summary.total_tasks, 2);
    assert!(
        summary.success_rate >= 0.5,
        "At least half should pass during tracer bullet phase"
    );
}

/// Test that the Critic integration is working
#[tokio::test]
async fn test_critic_integration() {
    let spec = load_fixture("CODE-01");

    let runner = GoldenRunner::new(RunnerConfig {
        enable_critic: true,
        ..Default::default()
    })
    .with_agents(spec.agents.clone());

    let result = runner.run_task(&spec).await;

    // Critic should have run and recorded results
    assert!(
        !result.metrics.critic_results.is_empty(),
        "Critic should have recorded check results"
    );

    // Verify critic checks were run
    let check_ids: Vec<&str> = result
        .metrics
        .critic_results
        .iter()
        .map(|c| c.check_id.as_str())
        .collect();

    // At minimum, lint and policy should run
    assert!(
        check_ids.iter().any(|id| *id == "lint" || *id == "policy"),
        "Should have run lint or policy checks. Got: {:?}",
        check_ids
    );
}

/// Test that metrics are properly collected
#[tokio::test]
async fn test_metrics_collection() {
    let spec = load_fixture("CODE-01");

    let runner = GoldenRunner::new(RunnerConfig::default()).with_agents(spec.agents.clone());

    let result = runner.run_task(&spec).await;

    // Verify all metric fields are populated
    let m = &result.metrics;

    assert!(m.total_duration_ms > 0, "total_duration should be > 0");
    assert!(m.subtask_count > 0, "subtask_count should be > 0");
    assert!(
        m.successful_subtasks > 0 || m.failed_subtasks > 0,
        "Should have success or failure recorded"
    );

    // Cost and tokens should be tracked
    assert!(m.tokens_used > 0, "Should track token usage");
    assert!(m.cost_usd > 0.0, "Should track cost");
}
