#![allow(clippy::disallowed_methods)]

use arkavo_mcp_tools::{
    Tool,
    time_sync::{GetAgentTimeTool, SyncAgentTimeTool},
};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

#[cfg(feature = "ntp-server")]
use arkavo_mcp_tools::ntp_server::NtpServer;

fn bench_get_agent_time_formats(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tool = GetAgentTimeTool::new();

    let formats = vec!["rfc3339", "unix", "iso8601"];

    let mut group = c.benchmark_group("get_agent_time_formats");
    for format in formats {
        group.bench_with_input(
            BenchmarkId::from_parameter(format),
            &format,
            |b, &format| {
                b.to_async(&runtime).iter(|| async {
                    tool.execute(black_box(json!({"format": format})))
                        .await
                        .unwrap()
                });
            },
        );
    }
    group.finish();
}

fn bench_get_agent_time_timezones(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let tool = GetAgentTimeTool::new();

    let timezones = vec!["UTC", "America/New_York", "Europe/London", "Asia/Tokyo"];

    let mut group = c.benchmark_group("get_agent_time_timezones");
    for tz in timezones {
        group.bench_with_input(BenchmarkId::from_parameter(tz), &tz, |b, &tz| {
            b.to_async(&runtime).iter(|| async {
                tool.execute(black_box(json!({
                    "format": "rfc3339",
                    "timezone": tz
                })))
                .await
                .unwrap()
            });
        });
    }
    group.finish();
}

#[cfg(feature = "ntp-server")]
fn bench_sync_with_orchestrator_ntp(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let addr = "127.0.0.1:19123".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    runtime.block_on(server.start()).unwrap();

    std::thread::sleep(Duration::from_millis(100));

    c.bench_function("sync_with_orchestrator_ntp", |b| {
        let tool = SyncAgentTimeTool::new();
        b.to_async(&runtime).iter(|| async {
            tool.execute(black_box(json!({
                "server": "127.0.0.1:19123",
                "protocol": "sntp"
            })))
            .await
            .unwrap()
        });
    });

    runtime.block_on(server.stop());
}

#[cfg(feature = "ntp-server")]
fn bench_concurrent_agent_sync(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let addr = "127.0.0.1:19124".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    runtime.block_on(server.start()).unwrap();

    std::thread::sleep(Duration::from_millis(100));

    let agent_counts = vec![10, 50, 100];

    let mut group = c.benchmark_group("concurrent_agent_sync");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(20));

    for &count in &agent_counts {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.to_async(&runtime).iter(|| async move {
                let semaphore = Arc::new(Semaphore::new(50));
                let mut handles = Vec::new();

                for _ in 0..count {
                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    let tool = SyncAgentTimeTool::new();

                    let handle = tokio::spawn(async move {
                        let result = tool
                            .execute(json!({
                                "server": "127.0.0.1:19124",
                                "protocol": "sntp"
                            }))
                            .await;
                        drop(permit);
                        result
                    });

                    handles.push(handle);
                }

                for handle in handles {
                    let _ = handle.await;
                }
            });
        });
    }
    group.finish();

    runtime.block_on(server.stop());
}

#[cfg(feature = "ntp-server")]
fn bench_orchestrator_throughput(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let addr = "127.0.0.1:19125".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    runtime.block_on(server.start()).unwrap();

    std::thread::sleep(Duration::from_millis(100));

    let operation_counts = vec![100, 500, 1000];

    let mut group = c.benchmark_group("orchestrator_throughput");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for &count in &operation_counts {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.to_async(&runtime).iter(|| async move {
                let semaphore = Arc::new(Semaphore::new(100));
                let mut handles = Vec::new();

                for _ in 0..count {
                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    let tool = SyncAgentTimeTool::new();

                    let handle = tokio::spawn(async move {
                        let _ = tool
                            .execute(json!({
                                "server": "127.0.0.1:19125",
                                "protocol": "sntp"
                            }))
                            .await;
                        drop(permit);
                    });

                    handles.push(handle);
                }

                for handle in handles {
                    let _ = handle.await;
                }
            });
        });
    }
    group.finish();

    runtime.block_on(server.stop());
}

#[cfg(feature = "ntp-server")]
fn bench_state_lock_contention(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let addr = "127.0.0.1:19126".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    runtime.block_on(server.start()).unwrap();

    std::thread::sleep(Duration::from_millis(100));

    let tool = SyncAgentTimeTool::new();
    let sync_state = tool.last_sync_state();

    runtime.block_on(async {
        let _ = tool.execute(json!({"server": "127.0.0.1:19126"})).await;
    });

    c.bench_function("state_lock_contention", |b| {
        b.to_async(&runtime).iter(|| async {
            let mut handles = Vec::new();

            for _ in 0..100 {
                let state = Arc::clone(&sync_state);
                let handle = tokio::spawn(async move {
                    let guard = state.lock().unwrap();
                    let _ = guard.timestamp;
                    drop(guard);
                });
                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.await;
            }
        });
    });

    runtime.block_on(server.stop());
}

#[cfg(feature = "ntp-server")]
fn bench_server_response_latency(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let addr = "127.0.0.1:19127".parse().unwrap();
    let server = NtpServer::new(addr, 3);
    runtime.block_on(server.start()).unwrap();

    std::thread::sleep(Duration::from_millis(100));

    c.bench_function("server_response_latency", |b| {
        let tool = SyncAgentTimeTool::new();
        b.to_async(&runtime).iter(|| async {
            let start = std::time::Instant::now();
            let _ = tool
                .execute(black_box(json!({
                    "server": "127.0.0.1:19127",
                    "protocol": "sntp"
                })))
                .await;
            start.elapsed()
        });
    });

    runtime.block_on(server.stop());
}

#[cfg(feature = "ntp-server")]
criterion_group! {
    name = benches_with_server;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(20))
        .sample_size(50)
        .warm_up_time(Duration::from_secs(3));
    targets = bench_get_agent_time_formats,
              bench_get_agent_time_timezones,
              bench_sync_with_orchestrator_ntp,
              bench_concurrent_agent_sync,
              bench_orchestrator_throughput,
              bench_state_lock_contention,
              bench_server_response_latency
}

#[cfg(not(feature = "ntp-server"))]
criterion_group! {
    name = benches_without_server;
    config = Criterion::default()
        .measurement_time(Duration::from_secs(10))
        .sample_size(50)
        .warm_up_time(Duration::from_secs(2));
    targets = bench_get_agent_time_formats,
              bench_get_agent_time_timezones
}

#[cfg(feature = "ntp-server")]
criterion_main!(benches_with_server);

#[cfg(not(feature = "ntp-server"))]
criterion_main!(benches_without_server);
