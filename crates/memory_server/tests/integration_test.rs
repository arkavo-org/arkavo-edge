use actix_web::{test, web, App};
use memory_server::{
    api::{categorize_memory, get_memory, search_memory, store_memory},
    models::{CreateMemoryRequest, SearchMemoryRequest},
    storage::MemoryStorage,
};
use uuid::Uuid;

#[actix_rt::test]
async fn test_memory_lifecycle() {
    let storage = web::Data::new(MemoryStorage::new().await.unwrap());

    let app = test::init_service(
        App::new()
            .app_data(storage.clone())
            .route("/memory", web::post().to(store_memory))
            .route("/memory/search", web::post().to(search_memory))
            .route("/memory/{id}", web::get().to(get_memory)),
    )
    .await;

    let create_req = CreateMemoryRequest {
        content: "This is a test memory about Rust programming".to_string(),
        metadata: Some(serde_json::json!({
            "tags": ["rust", "programming", "test"]
        })),
        category: Some("programming".to_string()),
    };

    let req = test::TestRequest::post()
        .uri("/memory")
        .set_json(&create_req)
        .to_request();

    let resp: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    let memory_id = resp["id"].as_str().unwrap();

    let req = test::TestRequest::get()
        .uri(&format!("/memory/{}", memory_id))
        .to_request();

    let resp: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(resp["content"], "This is a test memory about Rust programming");
    assert_eq!(resp["category"], "programming");

    let search_req = SearchMemoryRequest {
        query: "Rust programming language".to_string(),
        limit: Some(10),
        category: None,
    };

    let req = test::TestRequest::post()
        .uri("/memory/search")
        .set_json(&search_req)
        .to_request();

    let resp: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert!(resp["total"].as_u64().unwrap() > 0);
    assert!(resp["results"].as_array().unwrap().len() > 0);
}

#[actix_rt::test]
async fn test_categorization() {
    let storage = web::Data::new(MemoryStorage::new().await.unwrap());

    let app = test::init_service(
        App::new()
            .app_data(storage.clone())
            .route("/memory", web::post().to(store_memory))
            .route("/memory/categorize", web::post().to(categorize_memory)),
    )
    .await;

    let memories = vec![
        ("Learn Rust async programming", "programming"),
        ("Debug the memory leak in the application", "programming"),
        ("Meeting with team about project deadline", "work"),
        ("Doctor appointment at 3 PM", "personal"),
    ];

    for (content, category) in memories {
        let create_req = CreateMemoryRequest {
            content: content.to_string(),
            metadata: None,
            category: Some(category.to_string()),
        };

        let req = test::TestRequest::post()
            .uri("/memory")
            .set_json(&create_req)
            .to_request();

        test::call_service(&app, req).await;
    }

    let categorize_req = serde_json::json!({
        "content": "Fix the bug in the authentication system"
    });

    let req = test::TestRequest::post()
        .uri("/memory/categorize")
        .set_json(&categorize_req)
        .to_request();

    let resp: serde_json::Value = test::call_and_read_body_json(&app, req).await;
    assert_eq!(resp["category"], "programming");
    assert!(resp["confidence"].as_f64().unwrap() > 0.0);
}