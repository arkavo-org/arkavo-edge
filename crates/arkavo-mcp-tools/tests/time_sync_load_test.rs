#[cfg(feature = "ntp-server")]
mod common;

#[cfg(feature = "ntp-server")]
use common::{run_concurrent_operations, AgentSimulator, PerformanceMetrics};
#[cfg(feature = "ntp-server")]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "ntp-server")]
use std::sync::Arc;
#[cfg(feature = "ntp-server")]
use std::time::Instant;

#[cfg(feature = "ntp-server")]
use arkavo_mcp_tools::ntp_server::NtpServer;

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn load_test_1000_concurrent_agents() {
    let addr = "127.0.0.1:21123".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("\n=== Load Test: 1000 Concurrent Agents ===");
    let agent_count = 1000;
    let max_concurrency = 500;

    let progress = Arc::new(AtomicUsize::new(0));
    let start_time = Instant::now();

    let results = run_concurrent_operations(agent_count, max_concurrency, {
        let progress = Arc::clone(&progress);
        move |i| {
            let progress = Arc::clone(&progress);
            async move {
                let agent = AgentSimulator::new(format!("agent-{}", i));
                let op_start = Instant::now();
                let result = agent.sync_with_server("127.0.0.1:21123").await;
                let elapsed = op_start.elapsed().as_secs_f64() * 1000.0;

                let count = progress.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 100 == 0 {
                    let total_elapsed = start_time.elapsed().as_secs_f64();
                    let rate = count as f64 / total_elapsed;
                    println!(
                        "Progress: {}/{} agents ({:.0} agents/sec)",
                        count, agent_count, rate
                    );
                }

                if result.is_ok() {
                    Ok(elapsed)
                } else {
                    Err(format!("Failed agent-{}", i))
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

    let metrics = PerformanceMetrics::from_latencies("1000_concurrent".to_string(), latencies);
    metrics.print_report();

    println!("\n=== Final Results ===");
    println!("Total time: {:.2}s", total_elapsed);
    println!(
        "Throughput: {:.1} agents/sec",
        agent_count as f64 / total_elapsed
    );
    println!(
        "Success rate: {:.1}%",
        (successful as f64 / agent_count as f64) * 100.0
    );
    println!("Failed: {}", failed);

    let stats = server.get_stats().await;
    println!("\nServer Statistics:");
    println!("  Requests received: {}", stats.requests_received);
    println!("  Requests served: {}", stats.requests_served);
    println!("  Errors: {}", stats.errors);

    assert!(successful >= agent_count * 95 / 100, "Too many failures");

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn load_test_10k_requests() {
    let addr = "127.0.0.1:21124".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("\n=== Load Test: 10,000 Total Requests ===");
    let rounds = 20;
    let agents_per_round = 500;
    let total_requests = rounds * agents_per_round;

    println!(
        "Configuration: {} rounds × {} agents = {} total requests",
        rounds, agents_per_round, total_requests
    );

    let overall_start = Instant::now();
    let mut total_successful = 0;
    let mut total_failed = 0;
    let mut all_latencies = Vec::new();

    for round in 0..rounds {
        let round_start = Instant::now();

        let results = run_concurrent_operations(agents_per_round, 250, move |i| async move {
            let agent = AgentSimulator::new(format!("r{}-agent-{}", round, i));
            let start = Instant::now();
            let result = agent.sync_with_server("127.0.0.1:21124").await;
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;

            if result.is_ok() {
                Ok(elapsed)
            } else {
                Err("Failed".to_string())
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
    let metrics = PerformanceMetrics::from_latencies("10k_requests".to_string(), all_latencies);

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

    let stats = server.get_stats().await;
    println!("\nServer Statistics:");
    println!("  Requests received: {}", stats.requests_received);
    println!("  Requests served: {}", stats.requests_served);
    println!("  Errors: {}", stats.errors);

    assert!(total_successful >= (total_requests * 95) / 100);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn load_test_rapid_fire() {
    let addr = "127.0.0.1:21125".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("\n=== Load Test: Rapid Fire (Max Speed) ===");
    let agent_count = 2000;
    let max_concurrency = 1000;

    println!(
        "Launching {} agents with {} max concurrency",
        agent_count, max_concurrency
    );

    let start_time = Instant::now();
    let progress = Arc::new(AtomicUsize::new(0));

    let results = run_concurrent_operations(agent_count, max_concurrency, {
        let progress = Arc::clone(&progress);
        move |i| {
            let progress = Arc::clone(&progress);
            async move {
                let agent = AgentSimulator::new(format!("agent-{}", i));
                let op_start = Instant::now();
                let result = agent.sync_with_server("127.0.0.1:21125").await;
                let elapsed = op_start.elapsed().as_secs_f64() * 1000.0;

                let count = progress.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 200 == 0 {
                    let total_elapsed = start_time.elapsed().as_secs_f64();
                    let rate = count as f64 / total_elapsed;
                    println!("{} agents completed ({:.0}/sec)", count, rate);
                }

                if result.is_ok() {
                    Ok(elapsed)
                } else {
                    Err(format!("Failed-{}", i))
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

    let metrics = PerformanceMetrics::from_latencies("rapid_fire".to_string(), latencies);
    metrics.print_report();

    println!("\n=== Peak Performance ===");
    println!("Total time: {:.2}s", total_elapsed);
    println!(
        "Peak throughput: {:.1} agents/sec",
        agent_count as f64 / total_elapsed
    );
    println!(
        "Success rate: {:.1}%",
        (successful as f64 / agent_count as f64) * 100.0
    );

    let stats = server.get_stats().await;
    println!("Server served: {} requests", stats.requests_served);

    assert!(successful >= agent_count * 90 / 100);

    server.stop().await;
}
