use arkavo_protocol::task_store::SqliteTaskStore;

#[tokio::test]
async fn test_task_store_creates_parent_directory() {
    // Create a unique test directory based on timestamp
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let test_dir = format!("target/test_task_store_dir_{}", timestamp);
    let db_path = std::path::PathBuf::from(&test_dir)
        .join(".arkavo")
        .join("test_tasks.db");

    // Verify the .arkavo directory doesn't exist yet
    assert!(!db_path.parent().unwrap().exists());

    // Create the task store
    let result = SqliteTaskStore::new(&db_path).await;

    // If we can't create the database, just verify the directory was created
    match result {
        Ok(_store) => {
            // Verify the .arkavo directory was created
            assert!(db_path.parent().unwrap().exists());
            assert!(db_path.exists());
        }
        Err(_) => {
            // Even if database creation fails, directory should have been created
            assert!(db_path.parent().unwrap().exists());
        }
    }

    // Clean up
    if std::path::Path::new(&test_dir).exists() {
        std::fs::remove_dir_all(&test_dir).unwrap();
    }
}

#[tokio::test]
async fn test_in_memory_task_store_works() {
    // Just verify in-memory store can be created
    let _store = SqliteTaskStore::new_in_memory().await.unwrap();
}
