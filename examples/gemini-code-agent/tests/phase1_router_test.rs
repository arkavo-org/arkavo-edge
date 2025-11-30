/// Phase 1 Checkpoint: Router Classification and Selection
///
/// Tests the arkavo-router with real coding tasks to validate:
/// - Task classification accuracy
/// - Model selection logic
/// - Cost estimation
/// - Routing metrics
///
/// Run with: cargo test -p arkavo -- --test phase1_router_test

use arkavo_router::{Router, TaskCategory};


#[tokio::test]
async fn test_frontend_task_routing() {
    let router = Router::new().await.expect("Failed to create router");

    let decision = router
        .classify("Create a responsive React card component with Tailwind CSS that displays a user profile")
        .await
        .expect("Routing failed");

    // Should route to Gemini Flash for frontend
    assert_eq!(decision.task_category, TaskCategory::FrontendUI);
    assert!(decision.confidence > 0.75);
    assert!(decision.reasoning.contains("WebDev"));

    println!("\n=== Frontend Task Routing ===");
    println!("Task: React component");
    println!("Category: {:?}", decision.task_category);
    println!("Model: {}", decision.recommended_model.name());
    println!("Confidence: {:.2}%", decision.confidence * 100.0);
    println!("Cost: ${:.4}", decision.estimated_cost_usd);
    println!("Reasoning: {}", decision.reasoning);
}

#[tokio::test]
async fn test_backend_task_routing() {
    let router = Router::new().await.expect("Failed to create router");

    let decision = router
        .classify("Write a Node.js Express REST API endpoint for user authentication with JWT tokens")
        .await
        .expect("Routing failed");

    // Should route to Gemini Pro for backend
    assert_eq!(decision.task_category, TaskCategory::BackendAPI);
    assert!(decision.confidence > 0.70);

    println!("\n=== Backend Task Routing ===");
    println!("Task: REST API endpoint");
    println!("Category: {:?}", decision.task_category);
    println!("Model: {}", decision.recommended_model.name());
    println!("Confidence: {:.2}%", decision.confidence * 100.0);
    println!("Cost: ${:.4}", decision.estimated_cost_usd);
    println!("Reasoning: {}", decision.reasoning);
}

#[tokio::test]
async fn test_code_search_routing() {
    let router = Router::new().await.expect("Failed to create router");

    let decision = router
        .classify("Find all uses of the authenticate function in the codebase")
        .await
        .expect("Routing failed");

    // Should route to local Gemma 4B
    assert_eq!(decision.task_category, TaskCategory::CodeSearch);
    assert_eq!(decision.estimated_cost_usd, 0.0); // Local = free

    println!("\n=== Code Search Routing ===");
    println!("Task: Find function uses");
    println!("Category: {:?}", decision.task_category);
    println!("Model: {}", decision.recommended_model.name());
    println!("Confidence: {:.2}%", decision.confidence * 100.0);
    println!("Cost: ${:.4}", decision.estimated_cost_usd);
    println!("Reasoning: {}", decision.reasoning);
}

#[tokio::test]
async fn test_security_task_routing() {
    let router = Router::new().await.expect("Failed to create router");

    let decision = router
        .classify("Scan the authentication module for security vulnerabilities")
        .await
        .expect("Routing failed");

    // Should route to local Gemma 4B for privacy
    assert_eq!(decision.task_category, TaskCategory::SecurityScan);
    assert_eq!(decision.estimated_cost_usd, 0.0);

    println!("\n=== Security Scan Routing ===");
    println!("Task: Security scan");
    println!("Category: {:?}", decision.task_category);
    println!("Model: {}", decision.recommended_model.name());
    println!("Confidence: {:.2}%", decision.confidence * 100.0);
    println!("Cost: ${:.4}", decision.estimated_cost_usd);
    println!("Reasoning: {}", decision.reasoning);
}

#[tokio::test]
async fn test_test_generation_routing() {
    let router = Router::new().await.expect("Failed to create router");

    let decision = router
        .classify("Generate comprehensive Jest tests for a TodoList React component")
        .await
        .expect("Routing failed");

    // Should route to Gemini Pro for comprehensive tests
    assert_eq!(decision.task_category, TaskCategory::TestGeneration);

    println!("\n=== Test Generation Routing ===");
    println!("Task: Generate tests");
    println!("Category: {:?}", decision.task_category);
    println!("Model: {}", decision.recommended_model.name());
    println!("Confidence: {:.2}%", decision.confidence * 100.0);
    println!("Cost: ${:.4}", decision.estimated_cost_usd);
    println!("Reasoning: {}", decision.reasoning);
}

#[tokio::test]
async fn test_routing_metrics() {
    let router = Router::new().await.expect("Failed to create router");

    // Run multiple routing decisions
    let tasks = vec![
        "Create a React component",
        "Write a REST API",
        "Find all TODO comments",
        "Scan for vulnerabilities",
        "Generate unit tests",
        "Write documentation",
        "Refactor the auth module",
        "Implement a search feature",
    ];

    for task in tasks {
        router.classify(task).await.expect("Routing failed");
    }

    let metrics = router.get_metrics().await;

    println!("\n=== Routing Metrics Summary ===");
    println!("Total routes: {}", metrics.total_routes);
    println!("Average confidence: {:.2}%", metrics.average_confidence * 100.0);
    println!("Total cost saved: ${:.4}", metrics.total_cost_saved);
    println!("Cost savings: {:.1}%", metrics.cost_savings_percent());
    println!("Local model usage: {:.1}%", metrics.local_model_usage_percent());
    println!("\nRoutes by category:");
    for (category, count) in &metrics.routes_by_category {
        println!("  {}: {}", category, count);
    }
    println!("\nRoutes by model:");
    for (model, count) in &metrics.routes_by_model {
        println!("  {}: {}", model, count);
    }

    assert!(metrics.total_routes >= 8);
    assert!(metrics.cost_savings_percent() > 0.0);
}
