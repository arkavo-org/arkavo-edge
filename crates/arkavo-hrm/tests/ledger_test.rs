use arkavo_hrm::{Conductor, ContextStrategy, InMemoryTaskStore};
use arkavo_memory::MemoryStorage;
use std::sync::Arc;

#[tokio::test]
async fn test_ledger_context_offloading() {
    // 1. Setup
    let store = InMemoryTaskStore::new();
    let memory_storage = Arc::new(
        MemoryStorage::new_test()
            .await
            .expect("Failed to create memory storage"),
    );

    // 2. Initialize Conductor with Ledger
    let conductor = Conductor::new(store).with_ledger(memory_storage);

    // 3. Prepare large context
    let large_context =
        "This is a large block of text that represents a git diff or log file.\n".repeat(100);
    let summary = "Git diff of main.rs";

    // 4. Test: Use Ledger Strategy
    let result = conductor
        .prepare_context_for_burst(&large_context, Some(summary), &ContextStrategy::Ledger)
        .await
        .expect("Failed to prepare context");

    // 5. Verify: Result should be a pointer, not the original text
    assert!(result.contains("[ARCHIVED:"));
    assert!(result.contains(summary));
    assert!(result.contains("- ID:"));
    assert_ne!(result, large_context);

    println!("Ledger Pointer: {}", result);

    // 6. Test: Use Other Strategy (should return original)
    let result_full = conductor
        .prepare_context_for_burst(&large_context, Some(summary), &ContextStrategy::Full)
        .await
        .expect("Failed to prepare context");

    assert_eq!(result_full, large_context);
}
