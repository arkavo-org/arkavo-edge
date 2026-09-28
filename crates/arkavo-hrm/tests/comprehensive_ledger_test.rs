#![allow(clippy::disallowed_methods)]

use arkavo_hrm::{Conductor, ContextStrategy, InMemoryTaskStore};
use arkavo_memory::{ContextLedger, MemoryStorage};
use std::sync::Arc;

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

    // Note: ContextRestoreTool now requires Arc<MemoryStorage> injection.
    // See test_context_restore_tool_with_path() for end-to-end tool testing.

    let conductor = Conductor::new(store).with_ledger(Arc::new(memory_storage));

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
        .prepare_context_for_burst(
            &massive_log,
            Some("Server Access Logs"),
            &ContextStrategy::Ledger,
        )
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
    let shared_storage = Arc::new(MemoryStorage::new_test().await.expect("Storage init"));
    let ledger = ContextLedger::new(shared_storage); // Ledger shares storage

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
    // Use threshold of 0 to test strategy enforcement without size limits
    let storage_for_conductor = Arc::new(MemoryStorage::new_test().await.expect("Storage 3"));
    let conductor_strat = Conductor::new(InMemoryTaskStore::new())
        .with_ledger(storage_for_conductor)
        .with_min_offload_threshold(0);

    let text = "Small text";

    // Test Ledger Strategy
    let res_ledger = conductor_strat
        .prepare_context_for_burst(text, Some("Small"), &ContextStrategy::Ledger)
        .await
        .unwrap();
    assert!(
        res_ledger.contains("[ARCHIVED:"),
        "Ledger strategy should offload"
    );

    // Test Full Strategy
    let res_full = conductor_strat
        .prepare_context_for_burst(text, Some("Small"), &ContextStrategy::Full)
        .await
        .unwrap();
    assert_eq!(res_full, text, "Full strategy should keep text as is");

    println!("Strategy Enforcement: PASSED");

    println!("\n=== All Integration Tests Passed ===");
}

/// Tests the ContextRestoreTool with shared storage injection for test isolation.
/// This verifies that the tool correctly shares storage with the ledger.
#[tokio::test]
async fn test_context_restore_tool_with_storage() {
    use arkavo_mcp_tools::context_control::ContextRestoreTool;
    use arkavo_mcp_tools::server::Tool;

    println!("\n=== Testing ContextRestoreTool with Storage Injection ===\n");

    // Create a unique test DB path
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("arkavo_tool_test_{timestamp}.db"));

    // Create storage at that specific path
    let storage = Arc::new(
        MemoryStorage::with_path(db_path.clone(), Default::default())
            .await
            .expect("Storage creation failed"),
    );

    // Offload context using the ledger
    let ledger = ContextLedger::new(storage.clone());
    let original_text = "Secret payload for tool test: { key: 'value-42' }";
    let pointer = ledger
        .offload(original_text, "Tool Test Data", "integration_test")
        .await
        .expect("Offload failed");

    // Extract UUID from pointer
    let start_idx = pointer.find("ID: ").unwrap() + 4;
    let end_idx = pointer.find(']').unwrap();
    let uuid_str = &pointer[start_idx..end_idx];

    // Create the tool with the SAME shared storage
    let tool = ContextRestoreTool::new(storage.clone());

    // Execute the tool
    let params = serde_json::json!({ "id": uuid_str });
    let result = tool.execute(params).await.expect("Tool execution failed");

    // Verify the restored content
    let restored = result.get("content").and_then(|v| v.as_str()).unwrap();
    assert_eq!(restored, original_text, "Tool should restore exact content");

    println!("Path injection test: PASSED");

    // Cleanup
    let _ = std::fs::remove_file(&db_path);
}

/// Tests auto-summarization when no explicit summary is provided.
/// Requires a local model to be available (llama-cpp feature enabled).
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
#[tokio::test]
#[ignore] // Requires local model - run with: cargo test --features llama-cpp -- --ignored
async fn test_auto_summarization() {
    use std::env;

    println!("\n=== Testing Auto-Summarization ===\n");

    // Skip if no model path is set
    let model_path = match env::var("ARKAVO_TORG_MODEL_PATH") {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Skipping test: ARKAVO_TORG_MODEL_PATH not set");
            return;
        }
    };

    let store = InMemoryTaskStore::new();
    let memory_storage = Arc::new(
        MemoryStorage::new_test()
            .await
            .expect("Failed to create memory storage"),
    );

    // Create conductor with summarizer
    let conductor = Conductor::new(store)
        .with_ledger(memory_storage)
        .with_summarizer(model_path)
        .await
        .expect("Failed to create conductor with summarizer");

    // Create a git diff-like content
    let git_diff = r#"diff --git a/src/auth.rs b/src/auth.rs
index 1234567..abcdefg 100644
--- a/src/auth.rs
+++ b/src/auth.rs
@@ -10,6 +10,15 @@ use crate::jwt::TokenValidator;
+fn validate_jwt_token(token: &str) -> Result<Claims, AuthError> {
+    let validator = TokenValidator::new();
+    validator.validate(token)
+        .map_err(|e| AuthError::InvalidToken(e.to_string()))
+}
+
 fn authenticate_user(username: &str, password: &str) -> Result<User, AuthError> {
     let user = find_user(username)?;
     verify_password(&user, password)?;
+
+    // Generate JWT token after successful authentication
+    let token = generate_token(&user)?;
+    Ok(user.with_token(token))
 }
"#;

    // Call prepare_context_for_burst with None - should auto-summarize
    let pointer = conductor
        .prepare_context_for_burst(git_diff, None, &ContextStrategy::Ledger)
        .await
        .expect("Failed to auto-summarize and offload");

    println!("Generated Pointer: {}", pointer);

    // Verify the pointer contains a meaningful summary
    assert!(pointer.contains("[ARCHIVED:"), "Should have archive marker");
    assert!(pointer.contains("- ID:"), "Should have ID");

    // The summary should be descriptive, not just "Empty content"
    let summary_part = pointer
        .strip_prefix("[ARCHIVED: ")
        .and_then(|s| s.split(" - ID:").next())
        .unwrap_or("");

    assert!(!summary_part.is_empty(), "Summary should not be empty");
    assert!(
        summary_part.len() >= 10,
        "Summary should be at least 10 characters, got: '{}'",
        summary_part
    );

    println!("Auto-generated summary: '{}'", summary_part);
    println!("Auto-summarization test: PASSED");
}
