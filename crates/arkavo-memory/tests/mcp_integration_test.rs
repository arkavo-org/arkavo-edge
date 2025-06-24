use arkavo_mcp::Tool;
use arkavo_memory::{
    mcp_tools::{CategorizeMemoryTool, GetMemoryTool, SearchMemoryTool, StoreMemoryTool},
    storage::MemoryStorage,
};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires Ollama to be running"]
async fn test_memory_lifecycle_via_mcp() {
    let storage = Arc::new(MemoryStorage::new().await.unwrap());

    // Test storing a memory
    let store_tool = StoreMemoryTool::new(storage.clone());
    let store_params = json!({
        "content": "This is a test memory about Rust programming",
        "metadata": {
            "tags": ["rust", "programming", "test"]
        },
        "category": "programming"
    });

    let store_result = store_tool.execute(store_params).await.unwrap();
    let memory_id = store_result["id"].as_str().unwrap();

    // Test getting the memory
    let get_tool = GetMemoryTool::new(storage.clone());
    let get_params = json!({
        "id": memory_id
    });

    let get_result = get_tool.execute(get_params).await.unwrap();
    assert_eq!(
        get_result["content"],
        "This is a test memory about Rust programming"
    );
    assert_eq!(get_result["category"], "programming");

    // Test searching memories
    let search_tool = SearchMemoryTool::new(storage.clone());
    let search_params = json!({
        "query": "Rust programming language",
        "limit": 10
    });

    let search_result = search_tool.execute(search_params).await.unwrap();
    assert!(search_result["total"].as_u64().unwrap() > 0);
    assert!(search_result["results"].as_array().unwrap().len() > 0);
}

#[tokio::test]
#[ignore = "requires Ollama to be running"]
async fn test_categorization_via_mcp() {
    let storage = Arc::new(MemoryStorage::new().await.unwrap());

    // Store some memories with categories
    let store_tool = StoreMemoryTool::new(storage.clone());
    let memories = vec![
        ("Learn Rust async programming", "programming"),
        ("Debug the memory leak in the application", "programming"),
        ("Meeting with team about project deadline", "work"),
        ("Doctor appointment at 3 PM", "personal"),
    ];

    for (content, category) in memories {
        let params = json!({
            "content": content,
            "category": category
        });
        store_tool.execute(params).await.unwrap();
    }

    // Test categorization
    let categorize_tool = CategorizeMemoryTool::new(storage.clone());
    let categorize_params = json!({
        "content": "Fix the bug in the authentication system"
    });

    let result = categorize_tool.execute(categorize_params).await.unwrap();
    assert_eq!(result["category"], "programming");
    assert!(result["confidence"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
#[ignore = "requires Ollama to be running"]
async fn test_mcp_tool_schemas() {
    let storage = Arc::new(MemoryStorage::new().await.unwrap());

    // Verify tool schemas
    let store_tool = StoreMemoryTool::new(storage.clone());
    assert_eq!(store_tool.schema().name, "store_memory");
    assert!(
        store_tool.schema().parameters["required"]
            .as_array()
            .unwrap()
            .contains(&json!("content"))
    );

    let search_tool = SearchMemoryTool::new(storage.clone());
    assert_eq!(search_tool.schema().name, "search_memory");
    assert!(
        search_tool.schema().parameters["required"]
            .as_array()
            .unwrap()
            .contains(&json!("query"))
    );

    let get_tool = GetMemoryTool::new(storage.clone());
    assert_eq!(get_tool.schema().name, "get_memory");
    assert!(
        get_tool.schema().parameters["required"]
            .as_array()
            .unwrap()
            .contains(&json!("id"))
    );

    let categorize_tool = CategorizeMemoryTool::new(storage);
    assert_eq!(categorize_tool.schema().name, "categorize_memory");
    assert!(
        categorize_tool.schema().parameters["required"]
            .as_array()
            .unwrap()
            .contains(&json!("content"))
    );
}
