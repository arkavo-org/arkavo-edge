#![allow(clippy::disallowed_methods)]

use arkavo_protocol::{A2aMcpBridge, a2a::A2aClient};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

async fn run_concurrent_a2a_requests<F, Fut>(
    count: usize,
    max_concurrency: usize,
    operation: F,
) -> Vec<Result<f64, String>>
where
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = Result<f64, String>> + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    let mut handles = Vec::new();

    for i in 0..count {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let fut = operation(i);

        let handle = tokio::spawn(async move {
            let result = fut.await;
            drop(permit);
            result
        });

        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(Err(format!("Task panicked: {}", e))),
        }
    }

    results
}

/// The wall-clock second as the time tool reads it (`Utc::now().timestamp()`
/// floors the same system clock).
fn unix_now() -> i64 {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the epoch");
    i64::try_from(since_epoch.as_secs()).expect("seconds since epoch fit in i64")
}

/// Assert every reported second falls inside the window read around the
/// calls. A fixed drift bound between sequential answers fails whenever the
/// calls themselves straddle more seconds than the bound, which a slow or
/// loaded runner does; the window bound holds however long the calls take
/// while still catching any agent whose clock disagrees with the system's.
fn assert_within_window(label: &str, timestamps: &[i64], before: i64, after: i64) {
    for (i, &ts) in timestamps.iter().enumerate() {
        assert!(
            (before..=after).contains(&ts),
            "{label} {i} reported {ts}, outside the observed window [{before}, {after}]"
        );
    }
}

fn calculate_statistics(latencies: &[f64]) -> (f64, f64, f64, f64) {
    if latencies.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }

    let mut sorted = latencies.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    let avg = sorted.iter().sum::<f64>() / sorted.len() as f64;

    let p95_idx = ((sorted.len() - 1) as f64 * 0.95).round() as usize;
    let p95 = sorted[p95_idx];

    (min, max, avg, p95)
}

#[tokio::test]
async fn test_a2a_remote_time_query() {
    let bridge = A2aMcpBridge::new().await.expect("Failed to create bridge");
    let client = A2aClient::with_mcp_bridge(bridge);

    let response = client
        .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
        .await
        .unwrap();

    assert!(response.success);
    assert!(response.error.is_none());
    assert!(response.result["unix_seconds"].is_number());
    assert_eq!(response.result["format"], "unix");
    assert_eq!(response.result["source"], "system_clock");
}

#[tokio::test]
async fn test_a2a_time_query_all_formats() {
    let bridge = A2aMcpBridge::new().await.expect("Failed to create bridge");
    let client = A2aClient::with_mcp_bridge(bridge);

    let formats = vec!["rfc3339", "unix", "iso8601"];

    for format in formats {
        let response = client
            .call_mcp_tool("get_agent_time", json!({"format": format}))
            .await
            .unwrap();

        assert!(response.success, "Format {} failed", format);
        assert_eq!(response.result["format"], format);
    }
}

#[tokio::test]
async fn test_a2a_time_query_with_timezones() {
    let bridge = A2aMcpBridge::new().await.expect("Failed to create bridge");
    let client = A2aClient::with_mcp_bridge(bridge);

    let timezones = vec!["UTC", "America/New_York", "Europe/London", "Asia/Tokyo"];
    let mut unix_timestamps = Vec::new();

    let before = unix_now();
    for tz in timezones {
        let response = client
            .call_mcp_tool(
                "get_agent_time",
                json!({"format": "rfc3339", "timezone": tz}),
            )
            .await
            .unwrap();

        assert!(response.success);
        assert_eq!(response.result["timezone"], tz);
        unix_timestamps.push(response.result["unix_seconds"].as_i64().unwrap());
    }

    let after = unix_now();

    // Every timezone must report the same instant: the UTC epoch seconds.
    assert_within_window("Timezone query", &unix_timestamps, before, after);
}

#[tokio::test]
async fn test_a2a_multi_agent_time_consistency() {
    let agent_count = 10;
    let mut agents = Vec::new();

    for _ in 0..agent_count {
        let bridge = A2aMcpBridge::new().await.expect("Failed to create bridge");
        agents.push(A2aClient::with_mcp_bridge(bridge));
    }

    let mut unix_times = Vec::new();
    let before = unix_now();
    for agent in &agents {
        let response = agent
            .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
            .await
            .unwrap();

        assert!(response.success);
        unix_times.push(response.result["unix_seconds"].as_i64().unwrap());
    }

    let after = unix_now();

    assert_within_window("Agent", &unix_times, before, after);
}

#[tokio::test]
async fn test_a2a_concurrent_time_requests() {
    let agent_count = 100;
    let max_concurrency = 50;

    println!(
        "Starting concurrent A2A time requests: {} agents",
        agent_count
    );
    let test_start = Instant::now();

    let results = run_concurrent_a2a_requests(agent_count, max_concurrency, |i| async move {
        let bridge = A2aMcpBridge::new().await.expect("Failed to create bridge");
        let client = A2aClient::with_mcp_bridge(bridge);

        let start = Instant::now();
        let response = client
            .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
            .await;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        match response {
            Ok(r) if r.success => Ok(elapsed),
            Ok(r) => Err(format!(
                "Agent {} failed: {:?}",
                i,
                r.error.unwrap_or_default()
            )),
            Err(e) => Err(format!("Agent {} error: {}", i, e)),
        }
    })
    .await;

    let test_elapsed = test_start.elapsed().as_secs_f64();
    let successful = results.iter().filter(|r| r.is_ok()).count();
    let failed = results.len() - successful;

    println!(
        "Completed in {:.2}s - Successful: {}/{} ({:.1}%)",
        test_elapsed,
        successful,
        agent_count,
        (successful as f64 / agent_count as f64) * 100.0
    );

    let latencies: Vec<f64> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .copied()
        .collect();

    if !latencies.is_empty() {
        let (min, max, avg, p95) = calculate_statistics(&latencies);
        println!(
            "Latency - Min: {:.2}ms, Max: {:.2}ms, Avg: {:.2}ms, P95: {:.2}ms",
            min, max, avg, p95
        );
    }

    assert!(
        successful >= agent_count * 95 / 100,
        "Too many failures: {}/{}",
        failed,
        agent_count
    );
}

#[tokio::test]
async fn test_a2a_get_time_status() {
    let bridge = A2aMcpBridge::new().await.expect("Failed to create bridge");
    let client = A2aClient::with_mcp_bridge(bridge);

    let response = client
        .call_mcp_tool("get_time_status", json!({}))
        .await
        .unwrap();

    assert!(response.success);
    assert!(response.result["synchronized"].is_boolean());
    assert!(response.result["health"].is_string());
}

#[tokio::test]
async fn test_a2a_invalid_tool_request() {
    let bridge = A2aMcpBridge::new().await.expect("Failed to create bridge");
    let client = A2aClient::with_mcp_bridge(bridge);

    let response = client
        .call_mcp_tool("nonexistent_tool", json!({}))
        .await
        .unwrap();

    assert!(!response.success);
    assert!(response.error.is_some());
    assert!(response.error.unwrap().contains("not found"));
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[tokio::test]
async fn test_a2a_tool_latency_overhead() {
    // Both calls take microseconds, so a single sample of each is dominated by
    // scheduler noise on a shared runner. Compare medians of interleaved
    // samples against an absolute budget rather than a ratio of two samples.
    const WARMUP: usize = 8;
    const SAMPLES: usize = 64;
    const OVERHEAD_BUDGET: Duration = Duration::from_millis(5);

    let direct = A2aMcpBridge::new().await.expect("Failed to create bridge");
    let client =
        A2aClient::with_mcp_bridge(A2aMcpBridge::new().await.expect("Failed to create bridge"));
    let request = || arkavo_protocol::McpToolRequest {
        tool_name: "get_agent_time".to_string(),
        params: json!({"format": "unix"}),
    };

    for _ in 0..WARMUP {
        direct.call_tool(request()).await;
        client
            .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
            .await
            .unwrap();
    }

    let mut direct_samples = Vec::with_capacity(SAMPLES);
    let mut a2a_samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        direct.call_tool(request()).await;
        direct_samples.push(start.elapsed());

        let start = Instant::now();
        client
            .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
            .await
            .unwrap();
        a2a_samples.push(start.elapsed());
    }

    let direct_median = median(direct_samples);
    let a2a_median = median(a2a_samples);
    let overhead = a2a_median.saturating_sub(direct_median);
    println!(
        "Direct median: {direct_median:?}, A2A median: {a2a_median:?}, Overhead: {overhead:?}"
    );

    assert!(
        overhead < OVERHEAD_BUDGET,
        "A2A median overhead {overhead:?} exceeds {OVERHEAD_BUDGET:?}"
    );
}

#[tokio::test]
async fn test_a2a_orchestrator_pattern() {
    println!("Testing orchestrator pattern: querying 10 agents");

    let agent_count = 10;
    let orchestrator = A2aMcpBridge::new().await.expect("Failed to create bridge");
    let mut agents = Vec::new();

    for i in 0..agent_count {
        let bridge = A2aMcpBridge::new().await.expect("Failed to create bridge");
        agents.push((format!("agent-{}", i), A2aClient::with_mcp_bridge(bridge)));
    }

    let mut agent_times = Vec::new();
    let before = unix_now();
    for (name, agent) in &agents {
        let response = agent
            .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
            .await
            .unwrap();

        assert!(response.success, "Agent {} failed", name);
        agent_times.push(response.result["unix_seconds"].as_i64().unwrap());
    }

    let orchestrator_response = orchestrator
        .call_tool(arkavo_protocol::McpToolRequest {
            tool_name: "get_agent_time".to_string(),
            params: json!({"format": "unix"}),
        })
        .await;

    let after = unix_now();

    assert!(orchestrator_response.success);
    let orchestrator_time = orchestrator_response.result["unix_seconds"]
        .as_i64()
        .unwrap();

    for (i, &agent_time) in agent_times.iter().enumerate() {
        println!(
            "Agent {}: drift {}s",
            i,
            (agent_time - orchestrator_time).abs()
        );
    }
    assert_within_window("Orchestrator", &[orchestrator_time], before, after);
    assert_within_window("Agent", &agent_times, before, after);
}
