mod common;

use arkavo_mcp_tools::{
    Tool, ToolRegistry,
    time_sync::{GetAgentTimeTool, GetTimeStatusTool, SyncAgentTimeTool},
};
use common::{AgentSimulator, calculate_drift_statistics, run_concurrent_operations};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

#[cfg(feature = "ntp-server")]
use arkavo_mcp_tools::ntp_server::NtpServer;

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_orchestrator_ntp_server_startup() {
    let addr = "127.0.0.1:18123".parse().unwrap();
    let server = NtpServer::new(addr, 3);

    assert!(server.start().await.is_ok());
    assert!(server.is_running());

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let stats = server.get_stats().await;
    assert_eq!(stats.requests_received, 0);
    assert_eq!(stats.errors, 0);

    server.stop().await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    assert!(!server.is_running());
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_single_agent_sync_to_orchestrator() {
    let addr = "127.0.0.1:18124".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let agent = AgentSimulator::new("agent-1".to_string());
    let result = agent.sync_with_server("127.0.0.1:18124").await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response["success"], true);
    assert!(response["offset_ms"].is_number());
    assert!(response["rtt_ms"].is_number());
    assert_eq!(response["applied"], false);

    let offset_ms = response["offset_ms"].as_f64().unwrap();
    assert!(offset_ms.abs() < 100.0);

    let stats = server.get_stats().await;
    assert_eq!(stats.requests_received, 1);
    assert_eq!(stats.requests_served, 1);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_multi_agent_time_consistency() {
    let addr = "127.0.0.1:18125".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let agent_count = 10;
    let mut agents = Vec::new();
    for i in 0..agent_count {
        agents.push(AgentSimulator::new(format!("agent-{}", i)));
    }

    let mut offsets = Vec::new();
    for agent in &agents {
        let result = agent.sync_with_server("127.0.0.1:18125").await;
        if let Ok(response) = result {
            if response["success"] == true {
                offsets.push(response["offset_ms"].as_f64().unwrap());
            }
        }
    }

    assert_eq!(offsets.len(), agent_count);

    let (mean, std_dev, max_drift) = calculate_drift_statistics(&offsets);
    println!(
        "Mean offset: {:.3}ms, Std dev: {:.3}ms, Max drift: {:.3}ms",
        mean, std_dev, max_drift
    );

    assert!(max_drift < 10.0);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_concurrent_agent_registration() {
    let addr = "127.0.0.1:18126".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let agent_count = 100;
    let max_concurrency = 100;

    println!(
        "Starting concurrent agent registration test with {} agents",
        agent_count
    );
    let test_start = Instant::now();

    let results = run_concurrent_operations(agent_count, max_concurrency, |i| async move {
        if i % 10 == 0 {
            println!("Spawning agent batch starting at {}", i);
        }
        let agent = AgentSimulator::new(format!("agent-{}", i));
        let start = Instant::now();
        let result = agent.sync_with_server("127.0.0.1:18126").await;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        if result.is_ok() {
            Ok(elapsed)
        } else {
            Err(format!("Sync failed for agent-{}", i))
        }
    })
    .await;

    let test_elapsed = test_start.elapsed().as_secs_f64();
    let successful = results.iter().filter(|r| r.is_ok()).count();
    println!(
        "Completed in {:.2}s - Successful syncs: {}/{}",
        test_elapsed, successful, agent_count
    );

    let latencies: Vec<f64> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .copied()
        .collect();
    if !latencies.is_empty() {
        let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;
        println!("Average latency: {:.2}ms", avg);
    }

    assert!(successful >= agent_count * 95 / 100);

    let stats = server.get_stats().await;
    println!(
        "Server stats - Requests: {}, Served: {}, Errors: {}",
        stats.requests_received, stats.requests_served, stats.errors
    );

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_coordinator_sync_workflow() {
    let addr = "127.0.0.1:18127".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let registry = ToolRegistry::new();
    let get_time = registry.get("get_agent_time").unwrap();
    let sync_time = registry.get("sync_agent_time").unwrap();
    let get_status = registry.get("get_time_status").unwrap();

    let time1 = get_time.execute(json!({"format": "unix"})).await;
    assert!(time1.is_ok());

    let sync_result = sync_time
        .execute(json!({
            "server": "127.0.0.1:18127",
            "protocol": "sntp"
        }))
        .await;

    assert!(sync_result.is_ok());
    let response = sync_result.unwrap();
    assert_eq!(response["success"], true);

    let status = get_status.execute(json!({})).await;
    assert!(status.is_ok());
    let status_response = status.unwrap();
    assert_eq!(status_response["synchronized"], true);
    assert!(["healthy", "degraded"].contains(&status_response["health"].as_str().unwrap()));

    let time2 = get_time.execute(json!({"format": "rfc3339"})).await;
    assert!(time2.is_ok());

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_cross_timezone_coordination() {
    let addr = "127.0.0.1:18128".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let tool = GetAgentTimeTool::new();
    let timezones = vec!["UTC", "America/New_York", "Europe/London", "Asia/Tokyo"];

    let mut unix_timestamps = Vec::new();
    for tz in timezones {
        let result = tool
            .execute(json!({
                "format": "rfc3339",
                "timezone": tz
            }))
            .await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response["timezone"], tz);
        unix_timestamps.push(response["unix_seconds"].as_i64().unwrap());
    }

    let first = unix_timestamps[0];
    for &ts in &unix_timestamps[1..] {
        let diff = (ts - first).abs();
        assert!(diff <= 1);
    }

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_sync_state_propagation() {
    let addr = "127.0.0.1:18129".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let sync_tool = SyncAgentTimeTool::new();
    let sync_state = sync_tool.last_sync_state();
    let status_tool = GetTimeStatusTool::new(sync_state);

    let initial_status = status_tool.execute(json!({})).await.unwrap();
    assert_eq!(initial_status["synchronized"], false);

    let sync1 = sync_tool
        .execute(json!({"server": "127.0.0.1:18129"}))
        .await
        .unwrap();
    assert_eq!(sync1["success"], true);

    let status1 = status_tool.execute(json!({})).await.unwrap();
    assert_eq!(status1["synchronized"], true);
    assert!(status1["last_sync"].is_string());

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let sync2 = sync_tool
        .execute(json!({"server": "127.0.0.1:18129"}))
        .await
        .unwrap();
    assert_eq!(sync2["success"], true);

    let status2 = status_tool.execute(json!({})).await.unwrap();
    assert_eq!(status2["synchronized"], true);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_concurrent_status_queries() {
    let addr = "127.0.0.1:18130".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let sync_tool = SyncAgentTimeTool::new();
    let sync_state = sync_tool.last_sync_state();

    sync_tool
        .execute(json!({"server": "127.0.0.1:18130"}))
        .await
        .unwrap();

    let status_tool = GetTimeStatusTool::new(sync_state);

    let query_count = 100;
    let sync_state_ref = status_tool.get_sync_state();
    let results = run_concurrent_operations(query_count, 50, move |_| {
        let state = Arc::clone(&sync_state_ref);
        async move {
            let tool = GetTimeStatusTool::new(state);
            let start = Instant::now();
            let result = tool.execute(json!({})).await;
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;

            if result.is_ok() {
                Ok(elapsed)
            } else {
                Err("Status query failed".to_string())
            }
        }
    })
    .await;

    let successful = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(successful, query_count);

    server.stop().await;
}

#[cfg(feature = "ntp-server")]
#[tokio::test]
async fn test_orchestrator_server_stats() {
    let addr = "127.0.0.1:18131".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    server.start().await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    for i in 0..10 {
        let agent = AgentSimulator::new(format!("agent-{}", i));
        let _ = agent.sync_with_server("127.0.0.1:18131").await;
    }

    let stats = server.get_stats().await;
    println!(
        "Stats: requests={}, served={}, errors={}",
        stats.requests_received, stats.requests_served, stats.errors
    );

    assert!(stats.requests_received >= 10);
    assert!(stats.requests_served >= 9);

    server.stop().await;
}

#[tokio::test]
async fn test_all_time_tools_in_registry() {
    let registry = ToolRegistry::new();

    assert!(registry.get("get_agent_time").is_some());
    assert!(registry.get("sync_agent_time").is_some());
    assert!(registry.get("get_time_status").is_some());

    let categories = registry.list_by_category();
    assert!(categories.contains_key("System"));

    let system_tools = &categories["System"];
    let time_tools: Vec<_> = system_tools
        .iter()
        .filter(|t| t.name.contains("time") || t.name.contains("sync"))
        .collect();

    assert!(time_tools.len() >= 3);
}
