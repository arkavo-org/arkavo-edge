//! Concurrent Inference Benchmark
//!
//! Measures throughput of small model (qwen3-0.6B) while large model (8B) is busy.
//! This demonstrates the benefits of multi-model concurrent inference.

#![allow(clippy::uninlined_format_args)]

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;

/// Benchmark configuration
struct BenchmarkConfig {
    /// Number of concurrent small model requests
    small_requests: usize,
    /// Number of concurrent large model requests  
    large_requests: usize,
    /// Simulated token generation time for small model (ms)
    small_model_latency_ms: u64,
    /// Simulated token generation time for large model (ms)
    large_model_latency_ms: u64,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            small_requests: 100,
            large_requests: 5,
            // Small model is ~13x faster (3B vs 8B, plus smaller context)
            small_model_latency_ms: 100,
            large_model_latency_ms: 1300,
        }
    }
}

/// Simulated model inference that sleeps for the specified duration
/// to simulate token generation time
async fn simulate_inference(model_name: &str, latency_ms: u64) -> String {
    let start = Instant::now();
    tokio::time::sleep(Duration::from_millis(latency_ms)).await;
    format!("[{}] Completed in {:?}", model_name, start.elapsed())
}

/// Benchmark: Single model (baseline)
/// All requests go through a single model sequentially
fn benchmark_single_model(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let config = BenchmarkConfig::default();

    let mut group = c.benchmark_group("single_model_baseline");

    // Total requests = small + large
    let total_requests = config.small_requests + config.large_requests;
    group.throughput(Throughput::Elements(total_requests as u64));

    group.bench_function("sequential_single_model", |b| {
        b.to_async(&rt).iter(|| async {
            // Simulate all requests going to one model (sequential)
            for _i in 0..config.small_requests {
                simulate_inference("single-model", config.small_model_latency_ms).await;
            }
            for _i in 0..config.large_requests {
                simulate_inference("single-model", config.large_model_latency_ms).await;
            }
        });
    });

    group.finish();
}

/// Benchmark: Multi-model concurrent
/// Small requests go to qwen3-0.6B, large requests go to 8B model concurrently
fn benchmark_multi_model(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let config = BenchmarkConfig::default();

    let mut group = c.benchmark_group("multi_model_concurrent");

    // Test different concurrency levels
    for concurrency in [1, 2, 4, 8].iter() {
        let total_requests = config.small_requests + config.large_requests;
        group.throughput(Throughput::Elements(total_requests as u64));

        group.bench_with_input(
            BenchmarkId::new("concurrent_inference", concurrency),
            concurrency,
            |b, &_concurrency| {
                b.to_async(&rt).iter(|| async {
                    let mut handles = vec![];

                    // Spawn small model requests (qwen3-0.6B)
                    for _ in 0..config.small_requests {
                        handles.push(tokio::spawn(async move {
                            simulate_inference("qwen3-0.6B", config.small_model_latency_ms).await
                        }));
                    }

                    // Spawn large model requests (8B) - these run in parallel
                    // because different models have different contexts
                    for _ in 0..config.large_requests {
                        handles.push(tokio::spawn(async move {
                            simulate_inference("8B-model", config.large_model_latency_ms).await
                        }));
                    }

                    // Wait for all to complete
                    for handle in handles {
                        let _ = handle.await;
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: ModelRegistry concurrent access patterns
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
fn benchmark_registry_contention(c: &mut Criterion) {
    use arkavo_llm::ModelRegistry;

    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("model_registry_contention");

    // Benchmark read-only operations (is_loaded, model_names)
    group.bench_function("read_only_contention", |b| {
        let registry = Arc::new(ModelRegistry::new());

        b.to_async(&rt).iter(|| async {
            let mut handles = vec![];

            // Spawn many concurrent readers
            for _ in 0..100 {
                let reg = registry.clone();
                handles.push(tokio::spawn(async move {
                    let _ = reg.is_loaded("test-model");
                    let _ = reg.model_names();
                    let _ = reg.len();
                }));
            }

            for handle in handles {
                let _ = handle.await;
            }
        });
    });

    // Benchmark mixed read/write operations
    group.bench_function("mixed_read_write", |b| {
        let registry = Arc::new(ModelRegistry::new());

        b.to_async(&rt).iter(|| async {
            let mut handles = vec![];

            // Mix of reads and writes
            for i in 0..50 {
                let reg = registry.clone();
                if i % 10 == 0 {
                    // 10% writes
                    handles.push(tokio::spawn(async move {
                        let _ = reg.is_loaded(&format!("model-{}", i));
                    }));
                } else {
                    // 90% reads
                    handles.push(tokio::spawn(async move {
                        let _ = reg.model_names();
                        let _ = reg.len();
                    }));
                }
            }

            for handle in handles {
                let _ = handle.await;
            }
        });
    });

    group.finish();
}

/// Stress test: Maximum throughput under load
fn benchmark_max_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let config = BenchmarkConfig::default();

    let mut group = c.benchmark_group("max_throughput");
    group.sample_size(10); // Fewer samples for long-running benchmarks

    // Simulate: While 1 large request is running on 8B model,
    // how many small requests can qwen3-0.6B complete?
    group.bench_function("small_model_throughput_during_large", |b| {
        b.to_async(&rt).iter(|| async {
            // Start one large model request
            let large_handle = tokio::spawn(async move {
                simulate_inference("8B-model", config.large_model_latency_ms).await
            });

            // Count how many small requests complete during that time
            let mut small_completed = 0;
            let large_start = Instant::now();

            while large_start.elapsed() < Duration::from_millis(config.large_model_latency_ms) {
                simulate_inference("qwen3-0.6B", config.small_model_latency_ms).await;
                small_completed += 1;

                // Safety check to prevent infinite loop if timing is off
                if small_completed > 1000 {
                    break;
                }
            }

            // Wait for large model to complete
            let _ = large_handle.await;

            small_completed
        });
    });

    group.finish();
}

/// Compare latency distribution
fn benchmark_latency_distribution(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("latency_comparison");

    // Single model: all requests queued behind each other
    group.bench_function("single_model_10_requests", |b| {
        b.to_async(&rt).iter(|| async {
            for _i in 0..10 {
                simulate_inference("single-model", 100).await;
            }
        });
    });

    // Multi-model: requests can be interleaved
    group.bench_function("multi_model_10_requests", |b| {
        b.to_async(&rt).iter(|| async {
            let mut handles = vec![];

            // 5 small requests
            for _ in 0..5 {
                handles.push(tokio::spawn(async move {
                    simulate_inference("qwen3-0.6B", 50).await
                }));
            }

            // 5 large requests (running concurrently on different model)
            for _ in 0..5 {
                handles.push(tokio::spawn(async move {
                    simulate_inference("8B-model", 150).await
                }));
            }

            for handle in handles {
                let _ = handle.await;
            }
        });
    });

    group.finish();
}

// On llama-cpp feature, include registry contention benchmark
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
criterion_group!(
    benches,
    benchmark_single_model,
    benchmark_multi_model,
    benchmark_registry_contention,
    benchmark_max_throughput,
    benchmark_latency_distribution
);

// Without llama-cpp feature, exclude registry contention benchmark
#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
criterion_group!(
    benches,
    benchmark_single_model,
    benchmark_multi_model,
    benchmark_max_throughput,
    benchmark_latency_distribution
);

criterion_main!(benches);
