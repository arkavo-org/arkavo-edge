use arkavo_hrm::{Conductor, ContextStrategy, InMemoryTaskStore};
use arkavo_memory::MemoryStorage;

#[tokio::test]
async fn benchmark_monorepo_context_limit() {
    println!("Running Monorepo Context Benchmark...");

    // 1. Setup (Simulating the Monorepo Noise)
    // 50,000 tokens of error logs
    let error_line = "error[E0308]: mismatched types: expected `String`, found `integer`\n";
    let error_log = error_line.repeat(5000);

    let original_len = error_log.len();
    println!("Generated Error Log: {} bytes", original_len);

    let store = InMemoryTaskStore::new();
    let memory = MemoryStorage::new_test().await.expect("Memory init");
    let conductor = Conductor::new(store).with_ledger(memory);

    // 2. Execute Offload
    let start = std::time::Instant::now();
    let ptr = conductor
        .prepare_context_for_burst(&error_log, "Build Errors", &ContextStrategy::Ledger)
        .await
        .expect("Offload failed");
    let duration = start.elapsed();

    // 3. Assertions
    println!("Pointer: {}", ptr);
    println!("Pointer Len: {}", ptr.len());
    println!("Duration: {:?}", duration);

    // Peak context should be the pointer size (~150 bytes) vs Original (massive)
    assert!(ptr.len() < 200, "Context pointer exceeded 200 bytes");
    assert!(ptr.contains("ARCHIVED"), "Context was not archived");

    // Verify reduction
    let reduction = 1.0 - (ptr.len() as f64 / original_len as f64);
    assert!(reduction > 0.99, "Context reduction was less than 99%");

    println!(
        "Benchmark Passed: Context reduced by {:.4}%",
        reduction * 100.0
    );
}
