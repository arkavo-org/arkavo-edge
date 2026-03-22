//! SEQ-005, SEQ-012: Tests against LearningModule and RoutingMetrics
//! for behavioral baseline and async monitoring gaps.

use arkavo_router::learning::{LearningConfig, LearningModule};
use arkavo_router::metrics::RoutingMetrics;
use arkavo_test_macros::spec;

/// SEQ-005: LearningModule tracks per-agent statistics via Thompson Sampling
/// but has no concept of "behavioral baseline" that captures typical
/// action sequences for a skill.
#[spec("SEQ-005")]
#[tokio::test]
async fn learning_module_has_no_baseline_capture() {
    let module = LearningModule::new();
    let stats = module.get_stats("test-agent").await;

    // Stats track success/failure counts, expected_value, std_dev
    // But no: typical action sequences, frequency distributions, max path lengths
    // SEQ-005 requires: BaselineBuilder that extracts graph patterns from N sessions
    let stats_str = format!("{stats:?}");
    assert!(
        stats_str.contains("baseline") || stats_str.contains("sequence"),
        "SEQ-005: agent stats should include behavioral baseline, \
         but current stats are: {stats_str}"
    );
}

/// SEQ-012: RoutingMetrics tracks global average latency but not
/// per-session or per-sequence anomaly scores.
#[spec("SEQ-012")]
#[test]
fn routing_metrics_has_no_per_session_tracking() {
    let mut metrics = RoutingMetrics::new();
    metrics.record_router_latency(10);
    metrics.record_router_latency(20);

    let summary = metrics.summary();

    // Summary has total_routes, avg_router_decision_ms, cost_savings
    // But no: per-session latency, anomaly score, threshold breach alerts
    // SEQ-012 requires: AsyncSequenceMonitor with per-action anomaly scoring
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

    // Cold start: Thompson samples from Beta(2,1) — optimistic
    let score = module.thompson_sample("brand-new-agent", None).await;
    assert!(score > 0.0);

    // SEQ-005 edge case: "Insufficient history for baseline →
    // conservative policy applied (stricter gates)"
    // Currently, cold start is optimistic, not strict.
}
