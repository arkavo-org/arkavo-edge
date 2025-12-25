use arkavo_hrm::{Conductor, ContextStrategy, InMemoryTaskStore};
use arkavo_memory::{ContextLedger, MemoryStorage};

/// Helper to estimate tokens (rough approximation: 4 chars per token)
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

#[tokio::test]
async fn test_comprehensive_ledger_capabilities() {
    println!("\n=== Starting Comprehensive Context Ledger Integration Test ===\n");

    // 1. Setup Environment
    let store = InMemoryTaskStore::new();
    let memory_storage = MemoryStorage::new_test()
        .await
        .expect("Failed to create memory storage");

    // Note: ContextRestoreTool now supports path injection via `with_path()`.
    // See test_context_restore_tool_with_path() for end-to-end tool testing.

    let conductor = Conductor::new(store).with_ledger(memory_storage);

    // ==================================================================================
    // CAPABILITY 1: MASSIVE CONTEXT REDUCTION
    // ==================================================================================
    println!("--- Testing Capability: Context Reduction ---");

    // Generate a "Massive" log file (approx 12KB)
    let log_entry =
        "[2025-12-24 10:00:00] INFO: Processing request ID 12345. Latency: 45ms. User: admin.\n";
    let massive_log = log_entry.repeat(200);
    let original_size = massive_log.len();
    let original_tokens = estimate_tokens(&massive_log);

    println!(
        "Original Context Size: {} bytes (~{} tokens)",
        original_size, original_tokens
    );

    let start_time = std::time::Instant::now();
    let pointer = conductor
        .prepare_context_for_burst(&massive_log, "Server Access Logs", &ContextStrategy::Ledger)
        .await
        .expect("Offload failed");
    let duration = start_time.elapsed();

    let pointer_size = pointer.len();
    let pointer_tokens = estimate_tokens(&pointer);

    println!(
        "Offloaded Context Size: {} bytes (~{} tokens)",
        pointer_size, pointer_tokens
    );
    println!("Time taken: {:?}", duration);

    let reduction_ratio = 1.0 - (pointer_size as f64 / original_size as f64);
    println!("Reduction Ratio: {:.2}%", reduction_ratio * 100.0);

    assert!(reduction_ratio > 0.95, "Context reduction should be > 95%");
    assert!(
        pointer.contains("[ARCHIVED: Server Access Logs"),
        "Pointer format incorrect"
    );

    // ==================================================================================
    // CAPABILITY 2: DATA INTEGRITY & ROUNDTRIP
    // ==================================================================================
    println!("\n--- Testing Capability: Integrity Roundtrip ---");

    // Extract ID from pointer
    // Format: [ARCHIVED: Summary - ID: uuid]
    let start_idx = pointer.find("ID: ").unwrap() + 4;
    let end_idx = pointer.find("]").unwrap();
    let id_str = &pointer[start_idx..end_idx];

    println!("Extracted Fragment ID: {}", id_str);

    // Create separate storage for integrity test
    let shared_storage = MemoryStorage::new_test().await.expect("Storage init");
    let ledger = ContextLedger::new(shared_storage); // Ledger consumes storage

    let original_text = "Critical configuration data: { 'secret': 'XY-99' }";
    let summary = "Config Secret";

    let ptr = ledger
        .offload(original_text, summary, "test")
        .await
        .expect("Offload");

    // Parse ID
    let start = ptr.find("ID: ").unwrap() + 4;
    let end = ptr.find("]").unwrap();
    let uuid_str = &ptr[start..end];

    // Restore
    let restored_text = ledger.restore(uuid_str).await.expect("Restore");

    assert_eq!(
        original_text, restored_text,
        "Restored text must match original exactly"
    );
    println!("Integrity Check: PASSED");

    // ==================================================================================
    // CAPABILITY 3: STRATEGY ENFORCEMENT
    // ==================================================================================
    println!("\n--- Testing Capability: Strategy Enforcement ---");
    // Re-create conductor with new storage
    let storage_for_conductor = MemoryStorage::new_test().await.expect("Storage 3");
    let conductor_strat =
        Conductor::new(InMemoryTaskStore::new()).with_ledger(storage_for_conductor);

    let text = "Small text";

    // Test Ledger Strategy
    let res_ledger = conductor_strat
        .prepare_context_for_burst(text, "Small", &ContextStrategy::Ledger)
        .await
        .unwrap();
    assert!(
        res_ledger.contains("[ARCHIVED:"),
        "Ledger strategy should offload"
    );

    // Test Full Strategy
    let res_full = conductor_strat
        .prepare_context_for_burst(text, "Small", &ContextStrategy::Full)
        .await
        .unwrap();
    assert_eq!(res_full, text, "Full strategy should keep text as is");

    println!("Strategy Enforcement: PASSED");

    println!("\n=== All Integration Tests Passed ===");
}

/// Tests the ContextRestoreTool with path injection for test isolation.
/// This verifies the fix for the DB path mismatch issue documented in the handover.
#[tokio::test]
async fn test_context_restore_tool_with_path() {
    use arkavo_mcp_tools::context_control::ContextRestoreTool;
    use arkavo_mcp_tools::server::Tool;

    println!("\n=== Testing ContextRestoreTool with Path Injection ===\n");

    // Create a unique test DB path
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("arkavo_tool_test_{timestamp}.db"));

    // Create storage at that specific path
    let storage = MemoryStorage::with_path(db_path.clone(), Default::default())
        .await
        .expect("Storage creation failed");

    // Offload context using the ledger
    let ledger = ContextLedger::new(storage);
    let original_text = "Secret payload for tool test: { key: 'value-42' }";
    let pointer = ledger
        .offload(original_text, "Tool Test Data", "integration_test")
        .await
        .expect("Offload failed");

    // Extract UUID from pointer
    let start_idx = pointer.find("ID: ").unwrap() + 4;
    let end_idx = pointer.find(']').unwrap();
    let uuid_str = &pointer[start_idx..end_idx];
    println!("Offloaded with ID: {uuid_str}");

    // Create the tool pointing to the SAME database path
    let tool = ContextRestoreTool::with_path(Some(db_path.clone()));

    // Execute the tool
    let params = serde_json::json!({ "id": uuid_str });
    let result = tool.execute(params).await.expect("Tool execution failed");

    // Verify the restored content
    let restored = result.get("content").and_then(|v| v.as_str()).unwrap();
    assert_eq!(restored, original_text, "Tool should restore exact content");

    println!("Tool restored: {restored}");
    println!("Path injection test: PASSED");

    // Cleanup
    let _ = std::fs::remove_file(&db_path);
}
