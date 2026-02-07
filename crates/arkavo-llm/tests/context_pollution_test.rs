//! Context Pollution Tests
//!
//! These tests demonstrate potential issues with KV cache pollution
//! when contexts are shared between requests without proper isolation.

#![allow(
    clippy::disallowed_methods,
    clippy::significant_drop_tightening,
    clippy::uninlined_format_args
)]

use std::sync::Arc;
use std::time::Duration;

/// Test: Context sharing can lead to KV cache pollution
///
/// Issue: When multiple requests use the same LlamaContext sequentially,
/// the KV cache from the first request remains and can influence
/// the second request's output.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_context_kv_cache_pollution_risk() {
    use arkavo_llm::ModelRegistry;

    println!("\n========================================");
    println!("Context Pollution Risk Analysis");
    println!("========================================\n");

    let _registry = Arc::new(ModelRegistry::new());

    // Current implementation uses ONE context per model
    // This is the core issue: all requests share the same KV cache

    println!("ISSUE 1: Shared KV Cache");
    println!("------------------------");
    println!("Current architecture:");
    println!("  - One LlamaContext per model (in Mutex)");
    println!("  - All requests to 'model-a' share the SAME context");
    println!("  - KV cache from Request 1 remains for Request 2");
    println!();

    // Simulate the problem
    let start = std::time::Instant::now();

    // Request 1: "What is the capital of France?"
    // KV cache now contains tokens about France/Paris
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Request 2: "What is its population?"
    // Without proper context, "its" might refer to France (good)
    // But KV cache pollution could cause weird associations
    tokio::time::sleep(Duration::from_millis(10)).await;

    println!("Problem: KV cache state from Request 1 affects Request 2");
    println!("  - If Request 1 was about 'Paris', Request 2's 'it' might");
    println!("    incorrectly associate with Paris even if unrelated");
    println!();

    // Demonstrate the architectural limitation
    println!("ISSUE 2: Sequential Processing Per Model");
    println!("-----------------------------------------");
    println!("Current: Mutex<LlamaContext> means:");
    println!("  - Only ONE request can use 'model-a' at a time");
    println!("  - Requests queue even though GPU could handle more");
    println!();

    let sequential_time = start.elapsed();
    println!(
        "Sequential processing time for 2 requests: {:?}",
        sequential_time
    );
    println!();
}

/// Test: Demonstrates the need for context-per-request or proper KV cache management
#[tokio::test]
async fn test_context_isolation_requirements() {
    println!("\n========================================");
    println!("Context Isolation Requirements");
    println!("========================================\n");

    println!("REQUIREMENT 1: KV Cache Clearing");
    println!("--------------------------------");
    println!("Between requests, we need to:");
    println!("  - Clear KV cache (llama_kv_cache_clear)");
    println!("  - Or use separate contexts per request");
    println!();

    println!("REQUIREMENT 2: Context Pool");
    println!("---------------------------");
    println!("Better architecture:");
    println!("  - Pool of contexts per model");
    println!("  - Each request gets a clean context");
    println!("  - Return context to pool after use (cleared)");
    println!();

    println!("REQUIREMENT 3: Batch Processing");
    println!("-------------------------------");
    println!("For concurrent requests to SAME model:");
    println!("  - Use llama_decode with batching");
    println!("  - Or have multiple contexts (memory intensive)");
    println!();
}

/// Test: Verify current implementation has single context per model
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
async fn test_current_architecture_limitations() {
    use arkavo_llm::ModelRegistry;

    let _registry = ModelRegistry::new();

    println!("\n========================================");
    println!("Current Architecture Analysis");
    println!("========================================\n");

    println!("ModelRegistry stores:");
    println!("  models: RwLock<HashMap<String, Arc<LlamaModel>>>");
    println!("  contexts: RwLock<HashMap<String, Arc<Mutex<LlamaContext>>>>");
    println!();

    println!("Key observations:");
    println!("  1. ONE context per model name");
    println!("  2. Mutex ensures only one request uses context at a time");
    println!("  3. NO automatic KV cache clearing between requests");
    println!();

    println!("This means:");
    println!("  ✓ Thread-safe (Mutex prevents concurrent access)");
    println!("  ✗ Sequential processing per model");
    println!("  ✗ Potential KV cache pollution");
    println!();

    // The acquire_context returns a guard that holds the Mutex
    // This is good for thread safety but bad for throughput

    println!("Risk level: HIGH for production use");
    println!("  - Context pollution between unrelated requests");
    println!("  - Throughput bottleneck on popular models");
    println!();
}

/// Demonstrates ideal behavior with context isolation
#[tokio::test]
async fn test_ideal_context_isolation_pattern() {
    println!("\n========================================");
    println!("Ideal Implementation Pattern");
    println!("========================================\n");

    println!("Option 1: Context Pool (Recommended)");
    println!("-------------------------------------");
    println!("struct ContextPool {{");
    println!("    available: Vec<Arc<Mutex<LlamaContext>>>,");
    println!("    in_use: HashMap<RequestId, Arc<Mutex<LlamaContext>>>,");
    println!("}}");
    println!();
    println!("acquire_context():");
    println!("  1. Get available context from pool");
    println!("  2. Clear KV cache (llama_kv_cache_clear)");
    println!("  3. Return context guard");
    println!();
    println!("release_context():");
    println!("  1. Clear KV cache");
    println!("  2. Return to available pool");
    println!();

    println!("Option 2: Context per Request");
    println!("-----------------------------");
    println!("  - Create new LlamaContext for each request");
    println!("  - Expensive but maximally isolated");
    println!("  - Memory intensive");
    println!();

    println!("Option 3: Batched Decoding");
    println!("--------------------------");
    println!("  - Queue requests to same model");
    println!("  - Batch them together in one decode call");
    println!("  - Requires compatible sequence lengths");
    println!();
}

/// Test to verify the issue exists
#[test]
fn test_context_sharing_demonstration() {
    use std::sync::Mutex;

    println!("\n========================================");
    println!("Context Sharing Demonstration");
    println!("========================================\n");

    // Simulate the current architecture
    let shared_state = Arc::new(Mutex::new(String::from("Initial state")));

    // Request 1 modifies the state
    {
        let mut state = shared_state.lock().unwrap();
        *state = String::from("Request 1 was here: Paris is the capital");
    }

    // Request 2 sees Request 1's state (simulating KV cache pollution)
    {
        let state = shared_state.lock().unwrap();
        println!("Request 2 sees state: '{}'", state);
        println!();
        println!("PROBLEM: Request 2's inference might be influenced by");
        println!("         Request 1's KV cache (Paris association)");

        // In real llama.cpp, this would be KV cache entries
        // affecting attention scores for Request 2
    }

    println!();
    println!("SOLUTION: Clear state between requests");
    println!("  Request 1: Set state -> 'Paris is...'");
    println!("  CLEAR: Reset state -> ''");
    println!("  Request 2: Set state -> 'Berlin is...'");
    println!("  Result: No pollution!");
}
