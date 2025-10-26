#![allow(clippy::disallowed_methods)]

use arkavo_protocol::{A2aMcpBridge, a2a::A2aClient};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
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

fn calculate_percentile(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let idx = ((sorted_values.len() - 1) as f64 * p).round() as usize;
    sorted_values[idx]
}

struct PerformanceMetrics {
    operation_name: String,
    total: usize,
    successful: usize,
    failed: usize,
    min_ms: f64,
    max_ms: f64,
    avg_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

impl PerformanceMetrics {
    fn from_latencies(operation_name: String, mut latencies: Vec<f64>) -> Self {
        let total = latencies.len();
        let successful = latencies.iter().filter(|&&l| l > 0.0).count();
        let failed = total - successful;

        if latencies.is_empty() {
            return Self {
                operation_name,
                total,
                successful,
                failed,
                min_ms: 0.0,
                max_ms: 0.0,
                avg_ms: 0.0,
                p50_ms: 0.0,
                p95_ms: 0.0,
                p99_ms: 0.0,
            };
        }

        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let min = latencies[0];
        let max = latencies[latencies.len() - 1];
        let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;
        let p50 = calculate_percentile(&latencies, 0.50);
        let p95 = calculate_percentile(&latencies, 0.95);
        let p99 = calculate_percentile(&latencies, 0.99);

        Self {
            operation_name,
            total,
            successful,
            failed,
            min_ms: min,
            max_ms: max,
            avg_ms: avg,
            p50_ms: p50,
            p95_ms: p95,
            p99_ms: p99,
        }
    }

    fn print_report(&self) {
        println!("\n=== Performance Report: {} ===", self.operation_name);
        println!("Total operations: {}", self.total);
        println!("  Successful: {}", self.successful);
        println!("  Failed: {}", self.failed);
        println!("Latency (ms):");
        println!("  Min: {:.2}", self.min_ms);
        println!("  Max: {:.2}", self.max_ms);
        println!("  Avg: {:.2}", self.avg_ms);
        println!("  P50: {:.2}", self.p50_ms);
        println!("  P95: {:.2}", self.p95_ms);
        println!("  P99: {:.2}", self.p99_ms);
    }
}

#[tokio::test]
#[ignore]
async fn load_test_1000_a2a_time_requests() {
    println!("\n=== Load Test: 1000 A2A Time Requests ===");
    let request_count = 1000;
    let max_concurrency = 500;

    let progress = Arc::new(AtomicUsize::new(0));
    let start_time = Instant::now();

    let results = run_concurrent_a2a_requests(request_count, max_concurrency, {
        let progress = Arc::clone(&progress);
        move |i| {
            let progress = Arc::clone(&progress);
            async move {
                let bridge = A2aMcpBridge::new();
                let client = A2aClient::with_mcp_bridge(bridge);

                let op_start = Instant::now();
                let result = client
                    .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
                    .await;
                let elapsed = op_start.elapsed().as_secs_f64() * 1000.0;

                let count = progress.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 100 == 0 {
                    let total_elapsed = start_time.elapsed().as_secs_f64();
                    let rate = count as f64 / total_elapsed;
                    println!(
                        "Progress: {}/{} requests ({:.0} req/sec)",
                        count, request_count, rate
                    );
                }

                match result {
                    Ok(r) if r.success => Ok(elapsed),
                    Ok(r) => Err(format!("Request {} failed: {:?}", i, r.error)),
                    Err(e) => Err(format!("Request {} error: {}", i, e)),
                }
            }
        }
    })
    .await;

    let total_elapsed = start_time.elapsed().as_secs_f64();
    let successful = results.iter().filter(|r| r.is_ok()).count();
    let failed = results.len() - successful;

    let latencies: Vec<f64> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .copied()
        .collect();

    let metrics = PerformanceMetrics::from_latencies("1000_a2a_requests".to_string(), latencies);
    metrics.print_report();

    println!("\n=== Final Results ===");
    println!("Total time: {:.2}s", total_elapsed);
    println!(
        "Throughput: {:.1} req/sec",
        request_count as f64 / total_elapsed
    );
    println!(
        "Success rate: {:.1}%",
        (successful as f64 / request_count as f64) * 100.0
    );
    println!("Failed: {}", failed);

    assert!(
        successful >= request_count * 95 / 100,
        "Too many failures: {}/{}",
        failed,
        request_count
    );
}

#[tokio::test]
#[ignore]
async fn load_test_10k_a2a_requests() {
    println!("\n=== Load Test: 10,000 A2A Requests ===");
    let rounds = 20;
    let requests_per_round = 500;
    let total_requests = rounds * requests_per_round;

    println!(
        "Configuration: {} rounds × {} requests = {} total",
        rounds, requests_per_round, total_requests
    );

    let overall_start = Instant::now();
    let mut total_successful = 0;
    let mut total_failed = 0;
    let mut all_latencies = Vec::new();

    for round in 0..rounds {
        let round_start = Instant::now();

        let results = run_concurrent_a2a_requests(requests_per_round, 250, move |i| async move {
            let bridge = A2aMcpBridge::new();
            let client = A2aClient::with_mcp_bridge(bridge);

            let start = Instant::now();
            let result = client
                .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
                .await;
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;

            match result {
                Ok(r) if r.success => Ok(elapsed),
                _ => Err(format!("r{}-req-{} failed", round, i)),
            }
        })
        .await;

        let round_elapsed = round_start.elapsed().as_secs_f64();
        let successful = results.iter().filter(|r| r.is_ok()).count();
        let failed = results.len() - successful;

        total_successful += successful;
        total_failed += failed;

        let latencies: Vec<f64> = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .copied()
            .collect();
        all_latencies.extend(latencies.iter());

        if !latencies.is_empty() {
            let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;
            println!(
                "Round {}/{}: {:.2}s, {} ok, {} fail, avg {:.2}ms",
                round + 1,
                rounds,
                round_elapsed,
                successful,
                failed,
                avg
            );
        }
    }

    let total_elapsed = overall_start.elapsed().as_secs_f64();
    let metrics = PerformanceMetrics::from_latencies("10k_a2a_requests".to_string(), all_latencies);

    println!("\n=== Final Results ===");
    metrics.print_report();
    println!("Total time: {:.2}s", total_elapsed);
    println!(
        "Throughput: {:.1} req/sec",
        total_requests as f64 / total_elapsed
    );
    println!("Total successful: {}", total_successful);
    println!("Total failed: {}", total_failed);
    println!(
        "Success rate: {:.1}%",
        (total_successful as f64 / total_requests as f64) * 100.0
    );

    assert!(total_successful >= (total_requests * 95) / 100);
}

#[tokio::test]
#[ignore]
async fn load_test_rapid_fire_a2a() {
    println!("\n=== Load Test: Rapid Fire A2A (Max Speed) ===");
    let request_count = 2000;
    let max_concurrency = 1000;

    println!(
        "Launching {} requests with {} max concurrency",
        request_count, max_concurrency
    );

    let start_time = Instant::now();
    let progress = Arc::new(AtomicUsize::new(0));

    let results = run_concurrent_a2a_requests(request_count, max_concurrency, {
        let progress = Arc::clone(&progress);
        move |i| {
            let progress = Arc::clone(&progress);
            async move {
                let bridge = A2aMcpBridge::new();
                let client = A2aClient::with_mcp_bridge(bridge);

                let op_start = Instant::now();
                let result = client
                    .call_mcp_tool("get_agent_time", json!({"format": "unix"}))
                    .await;
                let elapsed = op_start.elapsed().as_secs_f64() * 1000.0;

                let count = progress.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 200 == 0 {
                    let total_elapsed = start_time.elapsed().as_secs_f64();
                    let rate = count as f64 / total_elapsed;
                    println!("{} requests completed ({:.0}/sec)", count, rate);
                }

                match result {
                    Ok(r) if r.success => Ok(elapsed),
                    _ => Err(format!("Failed-{}", i)),
                }
            }
        }
    })
    .await;

    let total_elapsed = start_time.elapsed().as_secs_f64();
    let successful = results.iter().filter(|r| r.is_ok()).count();

    let latencies: Vec<f64> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .copied()
        .collect();

    let metrics = PerformanceMetrics::from_latencies("rapid_fire_a2a".to_string(), latencies);
    metrics.print_report();

    println!("\n=== Peak Performance ===");
    println!("Total time: {:.2}s", total_elapsed);
    println!(
        "Peak throughput: {:.1} req/sec",
        request_count as f64 / total_elapsed
    );
    println!(
        "Success rate: {:.1}%",
        (successful as f64 / request_count as f64) * 100.0
    );

    assert!(successful >= request_count * 90 / 100);
}
