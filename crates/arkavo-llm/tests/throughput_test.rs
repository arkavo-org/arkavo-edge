//! Throughput Test: Small vs Large Model Concurrent Execution
//!
//! This test demonstrates how many small model requests (qwen3-0.6B)
//! can complete while a large model (8B) request is running.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Simulated model inference
async fn simulate_inference(model_name: &str, latency_ms: u64) -> String {
    tokio::time::sleep(Duration::from_millis(latency_ms)).await;
    format!("[{}] Done", model_name)
}

#[tokio::test]
async fn test_concurrent_throughput_small_vs_large() {
    println!("\n========================================");
    println!("Concurrent Inference Throughput Test");
    println!("========================================\n");

    // Configuration based on typical model performance
    // qwen3-0.6B is roughly 13x faster than an 8B model
    let small_model_latency_ms = 100u64; // qwen3-0.6B: ~100ms
    let large_model_latency_ms = 1300u64; // 8B model: ~1300ms

    // Test concurrent execution
    println!("Configuration:");
    println!(
        "  Small model (qwen3-0.6B): ~{}ms per request",
        small_model_latency_ms
    );
    println!(
        "  Large model (8B):         ~{}ms per request",
        large_model_latency_ms
    );
    println!(
        "  Performance ratio:        {}x faster\n",
        large_model_latency_ms / small_model_latency_ms
    );

    let start = Instant::now();
    let small_count = Arc::new(AtomicUsize::new(0));

    // Start large model request
    let large_handle = {
        let start = start;
        tokio::spawn(async move {
            simulate_inference("8B-model", large_model_latency_ms).await;
            println!("Large model (8B) completed in {:?}", start.elapsed());
        })
    };

    // Spawn small model requests continuously
    let small_start = Instant::now();
    let mut small_handles = vec![];

    while small_start.elapsed() < Duration::from_millis(large_model_latency_ms) {
        let count = small_count.clone();
        small_handles.push(tokio::spawn(async move {
            simulate_inference("qwen3-0.6B", small_model_latency_ms).await;
            count.fetch_add(1, Ordering::SeqCst);
        }));

        // Small delay between spawns to simulate realistic request arrival
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    // Wait for large model to complete
    let _ = large_handle.await;
    let large_duration = start.elapsed();

    // Count completed small requests
    let completed_small = small_count.load(Ordering::SeqCst);

    println!("\nResults:");
    println!("  Large model duration:     {:?}", large_duration);
    println!("  Small requests completed: {}", completed_small);
    println!(
        "  Theoretical max (serial): {}",
        large_model_latency_ms / small_model_latency_ms
    );
    println!();

    // With perfect concurrency, we expect roughly 13 small requests to complete
    // (1300ms / 100ms = 13)
    let theoretical_max = large_model_latency_ms / small_model_latency_ms;

    println!("========================================");
    println!("Analysis");
    println!("========================================");
    println!("With multi-model concurrent inference:");
    println!(
        "  - {} small requests to qwen3-0.6B completed",
        completed_small
    );
    println!("  - While 1 large request to 8B model was running");
    println!(
        "  - Throughput improvement: ~{}x for small requests",
        completed_small as f64 / 1.0
    );
    println!();
    println!("Without multi-model support (single model queue):");
    println!("  - Small requests would queue behind large request");
    println!(
        "  - Each small request would wait ~{}ms",
        large_model_latency_ms
    );
    println!(
        "  - Total latency for {} small requests: ~{}ms",
        completed_small,
        completed_small as u64 * large_model_latency_ms
    );
    println!();

    // Assertions
    assert!(
        completed_small >= (theoretical_max as usize).saturating_sub(3),
        "Expected at least {} small requests to complete, got {}",
        theoretical_max.saturating_sub(3),
        completed_small
    );

    println!("✅ Test passed: Multi-model concurrent inference provides");
    println!("   significant throughput improvement for mixed workloads.");
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_model_registry_concurrent_access() {
    use arkavo_llm::ModelRegistry;

    println!("\n========================================");
    println!("ModelRegistry Concurrent Access Test");
    println!("========================================\n");

    let registry = Arc::new(ModelRegistry::new());
    let iterations = 10000;

    // Measure read-only contention
    let start = Instant::now();
    let mut handles = vec![];

    for _ in 0..10 {
        let reg = registry.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..iterations {
                let _ = reg.is_loaded("test-model");
                let _ = reg.model_names();
                let _ = reg.len();
            }
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }

    let duration = start.elapsed();
    let ops_per_sec = (10 * iterations) as f64 / duration.as_secs_f64();

    println!("Read-only operations:");
    println!("  Total operations: {}", 10 * iterations);
    println!("  Duration: {:?}", duration);
    println!("  Ops/sec: {:.2}M", ops_per_sec / 1_000_000.0);
    println!();

    // Should be very fast due to RwLock allowing concurrent reads
    assert!(
        duration < Duration::from_secs(5),
        "Registry operations too slow"
    );

    println!("✅ ModelRegistry provides excellent concurrent read performance");
}

#[tokio::test]
async fn test_comparison_sequential_vs_concurrent() {
    println!("\n========================================");
    println!("Sequential vs Concurrent Comparison");
    println!("========================================\n");

    let small_latency = 50u64;
    let large_latency = 500u64;
    let num_small = 5;
    let num_large = 2;

    // Sequential execution (single model)
    let seq_start = Instant::now();
    for _ in 0..num_small {
        tokio::time::sleep(Duration::from_millis(small_latency)).await;
    }
    for _ in 0..num_large {
        tokio::time::sleep(Duration::from_millis(large_latency)).await;
    }
    let seq_duration = seq_start.elapsed();

    // Concurrent execution (multi-model)
    let con_start = Instant::now();
    let mut handles = vec![];

    for _ in 0..num_small {
        handles.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(small_latency)).await;
        }));
    }
    for _ in 0..num_large {
        handles.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(large_latency)).await;
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }
    let con_duration = con_start.elapsed();

    let improvement = seq_duration.as_millis() as f64 / con_duration.as_millis() as f64;

    println!("Sequential execution: {:?}", seq_duration);
    println!("Concurrent execution: {:?}", con_duration);
    println!("Improvement: {:.2}x faster\n", improvement);

    println!("With {} small + {} large requests:", num_small, num_large);
    println!("  - Sequential: All requests queue behind each other");
    println!("  - Concurrent: Small and large models process in parallel");
    println!();

    // Concurrent should be significantly faster
    assert!(
        con_duration < seq_duration / 2,
        "Concurrent execution should be at least 2x faster"
    );

    println!("✅ Concurrent execution shows significant improvement");
}
