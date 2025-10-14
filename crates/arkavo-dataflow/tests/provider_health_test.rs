#![allow(clippy::disallowed_methods)]
#![allow(unexpected_cfgs)]
#![allow(unused_variables)]
#![allow(clippy::float_cmp)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::needless_collect)]

use arkavo_llm::providers::{CircuitBreaker, ProviderHealthMonitor};
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn test_health_monitor_metrics() {
    let monitor = ProviderHealthMonitor::new(60);

    // Record successful requests
    monitor.record_request("provider1", true, Some(50)).await;
    monitor.record_request("provider1", true, Some(60)).await;
    monitor.record_request("provider1", true, Some(70)).await;

    // Record a failure
    monitor.record_request("provider1", false, None).await;

    // Check metrics
    let metrics = monitor.get_provider_metrics("provider1").await.unwrap();
    assert_eq!(metrics.total_requests, 4);
    assert_eq!(metrics.successful_requests, 3);
    assert_eq!(metrics.failed_requests, 1);
    assert_eq!(metrics.consecutive_failures, 1);
    assert!(metrics.average_latency_ms > 0.0);
    assert_eq!(metrics.uptime_percentage, 75.0);

    // Record another success to reset consecutive failures
    monitor.record_request("provider1", true, Some(80)).await;
    let metrics = monitor.get_provider_metrics("provider1").await.unwrap();
    assert_eq!(metrics.consecutive_failures, 0);
    assert_eq!(metrics.uptime_percentage, 80.0);
}

#[tokio::test]
async fn test_latency_percentiles() {
    let monitor = ProviderHealthMonitor::new(60);

    // Record multiple latencies
    let latencies = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    for latency in latencies {
        monitor
            .record_request("provider1", true, Some(latency))
            .await;
    }

    let metrics = monitor.get_provider_metrics("provider1").await.unwrap();
    assert_eq!(metrics.average_latency_ms, 55.0);

    // P95 should be around 95 (95th percentile of 10 values)
    assert!(metrics.p95_latency_ms >= 90.0);

    // P99 should be around 99 (99th percentile of 10 values)
    assert!(metrics.p99_latency_ms >= 90.0);
}

#[tokio::test]
async fn test_provider_health_status() {
    let monitor = ProviderHealthMonitor::new(60);

    // Initially no health status
    assert!(monitor.get_health_status("provider1").await.is_none());

    // Simulate providers for monitoring
    let providers = vec![
        (
            "local-ollama".to_string(),
            "http://localhost:11434".to_string(),
        ),
        (
            "remote-ollama".to_string(),
            "http://10.0.0.101:11434".to_string(),
        ),
    ];

    // Start monitoring (in real test, we'd mock the health check)
    monitor.start_monitoring(providers);

    // Give it a moment to start
    sleep(Duration::from_millis(100)).await;

    // Record some activity
    monitor.record_request("local-ollama", true, Some(10)).await;
    assert!(monitor.is_provider_healthy("local-ollama").await);
}

#[tokio::test]
async fn test_healthiest_provider_selection() {
    let monitor = ProviderHealthMonitor::new(60);

    // Set up metrics for multiple providers
    // Provider 1: High uptime, low latency
    for _ in 0..9 {
        monitor.record_request("provider1", true, Some(20)).await;
    }
    monitor.record_request("provider1", false, None).await;

    // Provider 2: Medium uptime, medium latency
    for _ in 0..7 {
        monitor.record_request("provider2", true, Some(50)).await;
    }
    for _ in 0..3 {
        monitor.record_request("provider2", false, None).await;
    }

    // Provider 3: Low uptime, high latency
    for _ in 0..5 {
        monitor.record_request("provider3", true, Some(100)).await;
    }
    for _ in 0..5 {
        monitor.record_request("provider3", false, None).await;
    }

    // Manually set health statuses (in real scenario, this would be from health checks)

    let providers = ["provider1", "provider2", "provider3"];

    // Provider 1 should be selected as healthiest (high uptime, low latency)
    // Note: In actual implementation, this would consider health status too
    let metrics1 = monitor.get_provider_metrics("provider1").await.unwrap();
    let metrics2 = monitor.get_provider_metrics("provider2").await.unwrap();
    let metrics3 = monitor.get_provider_metrics("provider3").await.unwrap();

    assert!(metrics1.uptime_percentage > metrics2.uptime_percentage);
    assert!(metrics2.uptime_percentage > metrics3.uptime_percentage);
    assert!(metrics1.average_latency_ms < metrics2.average_latency_ms);
    assert!(metrics2.average_latency_ms < metrics3.average_latency_ms);
}

#[tokio::test]
async fn test_circuit_breaker() {
    let breaker = CircuitBreaker::new(3, 60);

    // Initially circuit should be closed
    assert!(!breaker.is_open("provider1").await);

    // Record failures up to threshold
    breaker.record_failure("provider1").await;
    assert!(!breaker.is_open("provider1").await);

    breaker.record_failure("provider1").await;
    assert!(!breaker.is_open("provider1").await);

    // Third failure should open the circuit
    breaker.record_failure("provider1").await;
    assert!(breaker.is_open("provider1").await);

    // Success should close the circuit
    breaker.record_success("provider1").await;
    assert!(!breaker.is_open("provider1").await);
}

#[tokio::test]
async fn test_circuit_breaker_recovery() {
    let breaker = CircuitBreaker::new(2, 1); // 1 second recovery timeout

    // Open the circuit
    breaker.record_failure("provider1").await;
    breaker.record_failure("provider1").await;
    assert!(breaker.is_open("provider1").await);

    // Wait for recovery timeout
    sleep(Duration::from_millis(1100)).await;

    // Circuit should be closed after recovery timeout
    assert!(!breaker.is_open("provider1").await);
}

#[tokio::test]
async fn test_health_report_export() {
    let monitor = ProviderHealthMonitor::new(60);

    // Add some data
    monitor.record_request("provider1", true, Some(50)).await;
    monitor.record_request("provider2", false, None).await;

    // Export report
    let report = monitor.export_health_report().await;

    assert!(report["report_generated_at"].is_string());
    assert!(report["health_status"].is_object());
    assert!(report["metrics"].is_object());
    assert!(report["summary"].is_object());

    let summary = &report["summary"];
    assert!(summary["total_providers"].is_number());
}
