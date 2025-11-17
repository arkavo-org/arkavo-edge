use arkavo_bench::{ArkavoMode, ComparisonResult, SweBenchInstance};
use arkavo_router::Router;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
#[ignore] // Ignore by default since it requires API keys and network
async fn test_arkavo_mode_basic() {
    let router = Router::new().await;
    if router.is_err() {
        eprintln!("Skipping test: Router creation failed");
        return;
    }

    let router = Arc::new(router.unwrap());
    let arkavo_mode = ArkavoMode::new(router).await;

    if arkavo_mode.is_err() {
        eprintln!("Skipping test: Router initialization failed");
        return;
    }

    let arkavo_mode = arkavo_mode.unwrap();

    let instance = SweBenchInstance::new(
        "test-instance-1".to_string(),
        "https://github.com/test/repo".to_string(),
        "abc123".to_string(),
        "Fix the authentication bug where users cannot login with empty passwords".to_string(),
        Some("Check password validation logic".to_string()),
        "test.patch".to_string(),
    );

    let workspace = PathBuf::from("/tmp/test-workspace");
    std::fs::create_dir_all(&workspace).ok();

    let result = arkavo_mode.run_instance(&instance, &workspace).await;

    assert!(result.is_ok());
    let metrics = result.unwrap();
    assert_eq!(metrics.instance_id, "test-instance-1");
}

#[test]
fn test_comparison_result_metrics() {
    let raw_metrics = arkavo_bench::BenchMetrics::new("test-1-raw".to_string());
    let mut arkavo_metrics = arkavo_bench::BenchMetrics::new("test-1-arkavo".to_string());
    arkavo_metrics.set_resolved(true);

    let result = ComparisonResult {
        instance_id: "test-1".to_string(),
        raw_metrics,
        arkavo_metrics,
    };

    assert_eq!(result.improvement_percentage(), 100.0);
    assert_eq!(result.cost_difference_usd(), 0.0);
}

#[test]
fn test_bench_metrics_with_judgment() {
    use arkavo_router::{IssueType, JudgmentResult};

    let mut metrics = arkavo_bench::BenchMetrics::new("test-2".to_string());

    let judgment = JudgmentResult {
        passed: true,
        reason: Some("Quality gate passed".to_string()),
        issue_type: IssueType::None,
    };

    metrics.set_judgment(&judgment, 2);

    assert_eq!(metrics.quality_gate_passed, Some(true));
    assert_eq!(metrics.quality_retries, 2);
    assert_eq!(metrics.issue_type, Some("none".to_string()));
}
