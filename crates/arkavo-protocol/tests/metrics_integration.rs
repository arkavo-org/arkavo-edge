#[cfg(feature = "metrics")]
use arkavo_protocol::{MetricsCollector, RpcTimer};

#[tokio::test]
#[cfg(feature = "metrics")]
async fn test_metrics_collection() {
    // Initialize metrics
    let addr = "127.0.0.1:9090".parse().unwrap();
    MetricsCollector::init_prometheus(addr).expect("Failed to init prometheus");

    let collector = Arc::new(MetricsCollector::new(true));

    // Record some RPC requests
    collector.record_rpc_request("promise_request", true);
    collector.record_rpc_request("promise_request", false);
    collector.record_rpc_request("agent_discover", true);

    // Record rate limit blocks
    collector.record_rate_limit_blocked(None);
    collector.record_rate_limit_blocked(Some("192.168.1.1".parse().unwrap()));

    // Record mDNS discovery
    collector.record_mdns_discovery();

    // Record latency
    collector.record_rpc_latency("promise_request", Duration::from_millis(100));

    // Update gauge
    collector.update_rate_limit_entries(42);
}

#[tokio::test]
#[cfg(feature = "metrics")]
async fn test_rpc_timer_integration() {
    let collector = Arc::new(MetricsCollector::new(true));

    // Simulate a successful RPC
    {
        let timer = RpcTimer::new("test_method".to_string(), collector.clone());
        tokio::time::sleep(Duration::from_millis(10)).await;
        timer.success();
    }

    // Simulate a failed RPC
    {
        let timer = RpcTimer::new("test_method".to_string(), collector.clone());
        tokio::time::sleep(Duration::from_millis(5)).await;
        timer.error();
    }
}

#[test]
#[cfg(not(feature = "metrics"))]
fn test_metrics_disabled() {
    // When metrics feature is disabled, this test ensures the code compiles
    println!("Metrics feature is disabled");
}
