#[cfg(feature = "ntp-server")]
mod common;

#[cfg(feature = "ntp-server")]
use common::{run_concurrent_operations, AgentSimulator, PerformanceMetrics};
#[cfg(feature = "ntp-server")]
use std::time::{Duration, Instant};

#[cfg(feature = "ntp-server")]
use arkavo_mcp_tools::ntp_server::NtpServer;

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn stress_test_sustained_load() {
    let addr = "127.0.0.1:20123".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("Starting sustained load test: 1000 agents, 60 seconds");
    let test_duration = Duration::from_secs(60);
    let sync_interval = Duration::from_millis(100);
    let agent_count = 1000;

    let start_time = Instant::now();
    let mut total_syncs = 0;
    let mut successful_syncs = 0;
    let mut failed_syncs = 0;

    while start_time.elapsed() < test_duration {
        let results = run_concurrent_operations(agent_count, 100, |i| async move {
            let agent = AgentSimulator::new(format!("agent-{}", i));
            let result = agent.sync_with_server("127.0.0.1:20123").await;

            if result.is_ok() {
                Ok(1.0)
            } else {
                Err("Sync failed".to_string())
            }
        })
        .await;

        total_syncs += results.len();
        successful_syncs += results.iter().filter(|r| r.is_ok()).count();
        failed_syncs += results.iter().filter(|r| r.is_err()).count();

        tokio::time::sleep(sync_interval).await;
    }

    let elapsed = start_time.elapsed();
    let stats = server.get_stats().await;

    println!("\n=== Sustained Load Test Results ===");
    println!("Duration: {:.1}s", elapsed.as_secs_f64());
    println!("Total sync attempts: {}", total_syncs);
    println!("Successful: {}", successful_syncs);
    println!("Failed: {}", failed_syncs);
    println!(
        "Success rate: {:.2}%",
        (successful_syncs as f64 / total_syncs as f64) * 100.0
    );
    println!("Server requests: {}", stats.requests_received);
    println!("Server served: {}", stats.requests_served);
    println!("Server errors: {}", stats.errors);

    assert!(successful_syncs >= total_syncs * 95 / 100);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn stress_test_burst_traffic() {
    let addr = "127.0.0.1:20124".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("Starting burst traffic test: 10 → 1000 agents");

    let burst_start = Instant::now();
    let results = run_concurrent_operations(1000, 200, |i| async move {
        let agent = AgentSimulator::new(format!("agent-{}", i));
        let start = Instant::now();
        let result = agent.sync_with_server("127.0.0.1:20124").await;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        if result.is_ok() {
            Ok(elapsed)
        } else {
            Err(format!("Sync failed for agent-{}", i))
        }
    })
    .await;

    let burst_elapsed = burst_start.elapsed();

    let successful = results.iter().filter(|r| r.is_ok()).count();
    let failed = results.iter().filter(|r| r.is_err()).count();

    let latencies: Vec<f64> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .copied()
        .collect();

    let metrics = PerformanceMetrics::from_latencies("burst_traffic".to_string(), latencies);
    metrics.print_report();

    println!("\n=== Burst Traffic Test Results ===");
    println!("Burst duration: {:.3}s", burst_elapsed.as_secs_f64());
    println!("Total agents: 1000");
    println!("Successful: {}", successful);
    println!("Failed: {}", failed);
    println!(
        "Throughput: {:.1} agents/sec",
        1000.0 / burst_elapsed.as_secs_f64()
    );

    let stats = server.get_stats().await;
    println!("Server requests: {}", stats.requests_received);
    println!("Server served: {}", stats.requests_served);
    println!("Server errors: {}", stats.errors);

    assert!(successful >= 950);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn stress_test_agent_churn() {
    let addr = "127.0.0.1:20125".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("Starting agent churn test: agents continuously connect/disconnect for 30s");

    let test_duration = Duration::from_secs(30);
    let start_time = Instant::now();
    let mut agent_id_counter = 0;
    let mut total_connections = 0;
    let mut successful_syncs = 0;

    while start_time.elapsed() < test_duration {
        let batch_size = 50;
        let mut handles = Vec::new();

        for _ in 0..batch_size {
            let id = agent_id_counter;
            agent_id_counter += 1;

            let handle = tokio::spawn(async move {
                let agent = AgentSimulator::new(format!("agent-{}", id));
                agent.sync_with_server("127.0.0.1:20125").await
            });

            handles.push(handle);
        }

        for handle in handles {
            total_connections += 1;
            if let Ok(Ok(result)) = handle.await {
                if result["success"] == true {
                    successful_syncs += 1;
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    let stats = server.get_stats().await;

    println!("\n=== Agent Churn Test Results ===");
    println!("Duration: {:.1}s", start_time.elapsed().as_secs_f64());
    println!("Total connections: {}", total_connections);
    println!("Successful syncs: {}", successful_syncs);
    println!(
        "Success rate: {:.2}%",
        (successful_syncs as f64 / total_connections as f64) * 100.0
    );
    println!("Server requests: {}", stats.requests_received);
    println!("Server served: {}", stats.requests_served);

    assert!(successful_syncs >= total_connections * 95 / 100);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn stress_test_orchestrator_failover() {
    let addr = "127.0.0.1:20126".parse().unwrap();

    println!("Starting orchestrator failover test");

    let server1 = NtpServer::new(addr, 3);
    server1.start().await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let agent = AgentSimulator::new("agent-test".to_string());
    let result1 = agent.sync_with_server("127.0.0.1:20126").await;
    assert!(result1.is_ok());

    println!("Stopping orchestrator NTP server...");
    server1.stop().await;
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let result2 = agent.sync_with_server("127.0.0.1:20126").await;
    assert!(result2.is_err() || result2.unwrap()["success"] == false);

    println!("Restarting orchestrator NTP server...");
    let server2 = NtpServer::new(addr, 3);
    server2.start().await.unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let result3 = agent.sync_with_server("127.0.0.1:20126").await;
    assert!(result3.is_ok());
    assert_eq!(result3.unwrap()["success"], true);

    println!("Failover recovery successful!");

    server2.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn stress_test_rapid_sequential_syncs() {
    let addr = "127.0.0.1:20127".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("Starting rapid sequential sync test: 1000 syncs from single agent");

    let agent = AgentSimulator::new("agent-rapid".to_string());
    let mut latencies = Vec::new();
    let mut successful = 0;

    let start = Instant::now();
    for _ in 0..1000 {
        let sync_start = Instant::now();
        let result = agent.sync_with_server("127.0.0.1:20127").await;
        let elapsed = sync_start.elapsed().as_secs_f64() * 1000.0;

        if result.is_ok() && result.unwrap()["success"] == true {
            successful += 1;
            latencies.push(elapsed);
        }
    }
    let total_elapsed = start.elapsed();

    let metrics = PerformanceMetrics::from_latencies("rapid_sequential".to_string(), latencies);
    metrics.print_report();

    println!("\n=== Rapid Sequential Sync Results ===");
    println!("Total duration: {:.3}s", total_elapsed.as_secs_f64());
    println!("Successful syncs: {}/1000", successful);
    println!(
        "Throughput: {:.1} syncs/sec",
        1000.0 / total_elapsed.as_secs_f64()
    );

    assert!(successful >= 990);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
#[ignore]
async fn stress_test_memory_stability() {
    let addr = "127.0.0.1:20128".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    println!("Starting memory stability test: 10,000 syncs over 60 seconds");

    let rounds = 100;
    let syncs_per_round = 100;
    let mut total_successful = 0;

    for round in 0..rounds {
        let results = run_concurrent_operations(syncs_per_round, 50, |i| async move {
            let agent = AgentSimulator::new(format!("agent-{}-{}", round, i));
            let result = agent.sync_with_server("127.0.0.1:20128").await;

            if result.is_ok() {
                Ok(1.0)
            } else {
                Err("Failed".to_string())
            }
        })
        .await;

        total_successful += results.iter().filter(|r| r.is_ok()).count();

        if round % 10 == 0 {
            let stats = server.get_stats().await;
            println!(
                "Round {}/100: {} successful, server served: {}",
                round, total_successful, stats.requests_served
            );
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    let final_stats = server.get_stats().await;
    println!("\n=== Memory Stability Test Results ===");
    println!("Total syncs: 10,000");
    println!("Successful: {}", total_successful);
    println!("Server requests: {}", final_stats.requests_received);
    println!("Server served: {}", final_stats.requests_served);
    println!("Server errors: {}", final_stats.errors);

    assert!(total_successful >= 9500);

    server.stop().await;
}
