//! SEQ-005, SEQ-012: Tests against LearningModule and RoutingMetrics
//! for behavioral baseline and async monitoring gaps.

use arkavo_router::learning::LearningModule;
use arkavo_router::metrics::RoutingMetrics;
use arkavo_test_macros::spec;

/// SEQ-005: LearningModule tracks per-agent statistics but has no baseline.
/// Tripwire: when baseline capture is added, this will stop panicking.
#[spec("SEQ-005")]
#[tokio::test]
#[should_panic(expected = "SEQ-005")]
async fn learning_module_has_no_baseline_capture() {
    let module = LearningModule::new();
    let stats = module.get_stats("test-agent").await;

    let stats_str = format!("{stats:?}");
    assert!(
        stats_str.contains("baseline") || stats_str.contains("sequence"),
        "SEQ-005: agent stats should include behavioral baseline, \
         but current stats are: {stats_str}"
    );
}

/// SEQ-012: RoutingMetrics tracks global averages but not per-session anomaly.
/// Tripwire: when per-session tracking is added, this will stop panicking.
#[spec("SEQ-012")]
#[test]
#[should_panic(expected = "SEQ-012")]
fn routing_metrics_has_no_per_session_tracking() {
    let mut metrics = RoutingMetrics::new();
    metrics.record_router_latency(10);
    metrics.record_router_latency(20);

    let summary = metrics.summary();

    assert!(
        summary.contains("anomaly") || summary.contains("session"),
        "SEQ-012: metrics summary should include per-session anomaly tracking, \
         but current summary is: {summary}"
    );
}

/// SEQ-005: LearningModule cold start uses optimistic priors (Beta(2,1))
/// but has no "strict mode" for when no baseline is available.
#[spec("SEQ-005")]
#[tokio::test]
async fn learning_module_cold_start_has_no_strict_mode() {
    let module = LearningModule::new();

    let score = module.thompson_sample("brand-new-agent", None).await;
    assert!(score > 0.0);
}
