use arkavo_observability::metrics_snapshot::{
    MetricsSampler, MetricsSamplerConfig, MetricsSnapshot,
};
use tokio::sync::broadcast;

#[tokio::test]
async fn test_metrics_websocket_integration() {
    // Create a broadcast channel for metrics
    let (tx, mut rx) = broadcast::channel(10);

    // Start a simple metrics sampler
    let metrics_tx = tx.clone();
    tokio::spawn(async move {
        let config = MetricsSamplerConfig::default();
        let mut sampler = MetricsSampler::new(config);
        let metrics_collector = arkavo_observability::metrics::MetricsCollector::new();

        // Send one sample
        let snapshot = sampler.sample_metrics(&metrics_collector);
        let _ = metrics_tx.send(snapshot);
    });

    // Receive the metrics
    let snapshot = rx.recv().await.unwrap();

    // Verify snapshot is valid and small
    assert!(snapshot.estimated_json_size() <= 1024);
    assert_eq!(snapshot.active_sessions, 0);
    assert_eq!(snapshot.messages_per_second, 0.0);
}

#[test]
fn test_metrics_json_serialization() {
    let snapshot = MetricsSnapshot::new();

    // Add some realistic data
    let mut snapshot = snapshot;
    snapshot.active_sessions = 10;
    snapshot.messages_per_second = 50.5;
    snapshot.error_rate = 0.1;
    snapshot.p95_latency_ms = 15.2;
    snapshot.p99_latency_ms = 25.8;

    // Serialize to JSON
    let json = serde_json::to_string(&snapshot).unwrap();
    println!("Metrics JSON ({} bytes): {}", json.len(), json);

    // Verify size is under 1KB
    assert!(json.len() <= 1024);

    // Verify deserialization works
    let deserialized: MetricsSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.active_sessions, snapshot.active_sessions);
}
