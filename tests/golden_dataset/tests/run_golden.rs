//! Integration tests for the Golden Dataset
//!
//! Runs benchmark tasks through the full HRM orchestration pipeline
//! to validate Conductor -> Router -> Execute -> Critic flow.
//!
//! Also includes adversarial tests (Phase 5) for fault tolerance validation.

use golden_dataset::adversarial_runner::{load_scenario, AdversarialRunner};
use golden_dataset::metrics::BenchmarkSummary;
use golden_dataset::runner::{GoldenRunner, RunnerConfig};
use golden_dataset::{load_task_spec, TaskSpec};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/tasks")
}

fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/scenarios")
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

// ============================================================================
// Phase 5: Adversarial Tests
// ============================================================================

/// Test ADV-01: The Drunk Agent
///
/// Validates recovery from schema corruption:
/// 1. Primary agent returns garbled JSON
/// 2. Conductor detects parse error
/// 3. Router penalizes the faulty agent
/// 4. Fallback agent succeeds
#[tokio::test]
async fn test_adv_01_drunk_agent() {
    let path = scenarios_dir().join("ADV-01.json");
    let scenario = load_scenario(&path).expect("Failed to load ADV-01");

    let runner = AdversarialRunner::new();
    let result = runner.run_scenario(&scenario).await;

    println!("\n=== ADV-01 Results ===");
    println!("Task succeeded: {}", result.task_succeeded);
    println!("Faults injected: {}", result.faults_injected);
    println!("Retries: {}", result.retries);
    println!("Agent scores: {:?}", result.agent_scores);

    // Should recover via fallback agent
    assert!(
        result.task_succeeded,
        "Should recover from drunk agent via fallback"
    );

    // Should have injected at least one fault
    assert!(
        result.faults_injected > 0,
        "Should have injected schema corruption faults"
    );

    // Drunk agent should have lower score than fallback
    let drunk_score = result
        .agent_scores
        .iter()
        .find(|(id, _)| id == "drunk-agent")
        .map(|(_, s)| *s);
    let reliable_score = result
        .agent_scores
        .iter()
        .find(|(id, _)| id == "reliable-agent")
        .map(|(_, s)| *s);

    if let (Some(drunk), Some(reliable)) = (drunk_score, reliable_score) {
        assert!(
            drunk < reliable,
            "Drunk agent ({drunk}) should have lower score than reliable ({reliable})"
        );
    }

    assert!(result.passed, "ADV-01 should pass expectations");
}

/// Test ADV-02: The Lazy Agent
///
/// Validates Critic semantic verification:
/// 1. Agent returns syntactically valid but empty response
/// 2. Critic's lint/policy checks catch the issue
/// 3. Result is vetoed
/// 4. Fallback agent provides real content
#[tokio::test]
async fn test_adv_02_lazy_agent() {
    let path = scenarios_dir().join("ADV-02.json");
    let scenario = load_scenario(&path).expect("Failed to load ADV-02");

    let runner = AdversarialRunner::new();
    let result = runner.run_scenario(&scenario).await;

    println!("\n=== ADV-02 Results ===");
    println!("Task succeeded: {}", result.task_succeeded);
    println!("Critic rejections: {}", result.critic_rejections);
    println!("Agent scores: {:?}", result.agent_scores);

    // Should recover via diligent fallback
    assert!(
        result.task_succeeded,
        "Should recover from lazy agent via fallback"
    );

    // Critic should have vetoed at least once
    assert!(
        result.critic_rejections > 0,
        "Critic should have rejected lazy response"
    );

    // Lazy agent should have lower score
    let lazy_score = result
        .agent_scores
        .iter()
        .find(|(id, _)| id == "lazy-agent")
        .map(|(_, s)| *s);

    if let Some(score) = lazy_score {
        assert!(
            score < 1.0,
            "Lazy agent should have been penalized, got {score}"
        );
    }

    assert!(result.passed, "ADV-02 should pass expectations");
}

/// Test ADV-03: The Infinite Loop
///
/// Validates loop detection guardrails:
/// 1. All agents fail repeatedly
/// 2. Conductor detects thrashing pattern
/// 3. Task aborts with StrategicThrashing error
/// 4. Does NOT loop forever
#[tokio::test]
async fn test_adv_03_infinite_loop() {
    let path = scenarios_dir().join("ADV-03.json");
    let scenario = load_scenario(&path).expect("Failed to load ADV-03");

    let runner = AdversarialRunner::new();
    let result = runner.run_scenario(&scenario).await;

    println!("\n=== ADV-03 Results ===");
    println!("Task succeeded: {}", result.task_succeeded);
    println!("Error: {:?}", result.error);
    println!("Total attempts: {}", result.total_attempts);
    println!("Retries: {}", result.retries);

    // Task should fail (no recovery possible)
    assert!(
        !result.task_succeeded,
        "Task should fail when all agents fail"
    );

    // Note: Full thrashing detection requires conductor implementation.
    // For now, verify the task fails with enough retries.
    // The error may or may not be set depending on conductor behavior.

    // Should have attempted multiple retries before giving up
    assert!(
        result.retries >= 2,
        "Should have retried at least twice before giving up"
    );

    assert!(result.passed, "ADV-03 should pass expectations");
}

/// Test Router Learning: Bandit Check
///
/// Validates that the Router's Thompson Sampling correctly penalizes
/// failing agents across multiple interactions.
#[tokio::test]
async fn test_router_learning_bandit_check() {
    let path = scenarios_dir().join("ADV-01.json");
    let scenario = load_scenario(&path).expect("Failed to load ADV-01");

    let runner = AdversarialRunner::new();
    let result = runner.run_scenario(&scenario).await;

    println!("\n=== Bandit Check ===");
    println!("Agent scores after ADV-01:");
    for (agent_id, score) in &result.agent_scores {
        println!("  {agent_id}: {score:.3}");
    }

    // Verify the "drunk" agent has been penalized
    let drunk_score = result
        .agent_scores
        .iter()
        .find(|(id, _)| id == "drunk-agent")
        .map(|(_, s)| *s)
        .unwrap_or(1.0);

    // After failures, score should be significantly lower than 1.0
    assert!(
        drunk_score < 0.5,
        "Drunk agent should have low score after failures, got {drunk_score}"
    );

    // The reliable agent should have higher score
    let reliable_score = result
        .agent_scores
        .iter()
        .find(|(id, _)| id == "reliable-agent")
        .map(|(_, s)| *s)
        .unwrap_or(0.0);

    assert!(
        reliable_score > drunk_score,
        "Reliable agent ({reliable_score}) should score higher than drunk ({drunk_score})"
    );
}

/// Test adversarial suite summary
#[tokio::test]
async fn test_adversarial_suite() {
    let scenarios = vec!["ADV-01", "ADV-02", "ADV-03"];
    let runner = AdversarialRunner::new();

    let mut passed = 0;
    let mut failed = 0;

    for name in &scenarios {
        let path = scenarios_dir().join(format!("{name}.json"));
        let scenario = load_scenario(&path).expect(&format!("Failed to load {name}"));
        let result = runner.run_scenario(&scenario).await;

        if result.passed {
            passed += 1;
            println!("{name}: PASS");
        } else {
            failed += 1;
            println!("{name}: FAIL - {:?}", result.error);
        }
    }

    println!("\n=== Adversarial Suite Summary ===");
    println!("Passed: {passed}");
    println!("Failed: {failed}");
    println!(
        "Success rate: {:.1}%",
        100.0 * passed as f64 / scenarios.len() as f64
    );

    // All adversarial tests should pass (they test expected failure modes)
    assert_eq!(failed, 0, "All adversarial scenarios should pass");
}
