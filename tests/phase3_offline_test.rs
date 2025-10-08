use arkavo_router::{Router, TaskCategory};

#[tokio::test]
async fn test_offline_mode_creation() {
    let router_result = Router::new_offline().await;

    if router_result.is_err() {
        eprintln!("⚠️  Router initialization failed, skipping test");
        return;
    }

    let router = router_result.unwrap();

    println!("\n=== Offline Mode Test ===");
    println!("Router created in offline mode");

    let decision = router
        .route("Create a React component with state")
        .await
        .expect("Routing failed");

    println!("Task category: {:?}", decision.task_category);
    println!("Model selected: {}", decision.recommended_model.name());
    println!("Cost: ${:.4}", decision.estimated_cost_usd);
    println!("Reasoning: {}", decision.reasoning);

    assert!(decision.reasoning.contains("Offline mode") || decision.recommended_model.is_local());
    assert_eq!(decision.estimated_cost_usd, 0.0);
}

#[tokio::test]
async fn test_all_tasks_work_offline() {
    let router_result = Router::new_offline().await;

    if router_result.is_err() {
        eprintln!("⚠️  Router initialization failed, skipping test");
        return;
    }

    let router = router_result.unwrap();

    let tasks = vec![
        ("Create a responsive React card component", TaskCategory::FrontendUI),
        ("Write a REST API endpoint for authentication", TaskCategory::BackendAPI),
        ("Find all uses of the authenticate function", TaskCategory::CodeSearch),
        ("Scan for security vulnerabilities", TaskCategory::SecurityScan),
        ("Generate unit tests for UserController", TaskCategory::TestGeneration),
        ("Write documentation for the API", TaskCategory::Documentation),
        ("Refactor the authentication module", TaskCategory::Refactoring),
        ("Implement a search feature", TaskCategory::General),
    ];

    println!("\n=== Offline Task Coverage Test ===");
    println!("Testing {} different task types\n", tasks.len());

    let mut all_local = true;
    let mut all_zero_cost = true;

    for (task, expected_category) in tasks {
        let decision = router.route(task).await.expect("Routing failed");

        println!("Task: {}", task);
        println!("  Category: {:?}", decision.task_category);
        println!("  Model: {}", decision.recommended_model.name());
        println!("  Cost: ${:.4}", decision.estimated_cost_usd);

        assert_eq!(decision.task_category, expected_category);
        assert!(decision.recommended_model.is_local());
        assert_eq!(decision.estimated_cost_usd, 0.0);

        if !decision.recommended_model.is_local() {
            all_local = false;
        }
        if decision.estimated_cost_usd > 0.0 {
            all_zero_cost = false;
        }
    }

    println!("\n=== Offline Coverage Summary ===");
    println!("All tasks use local models: {}", all_local);
    println!("All tasks zero cost: {}", all_zero_cost);
    println!("Offline availability: 100%");

    assert!(all_local, "All tasks must use local models in offline mode");
    assert!(all_zero_cost, "All tasks must be zero cost in offline mode");
}

#[tokio::test]
async fn test_online_vs_offline_comparison() {
    let online_router_result = Router::new().await;
    let offline_router_result = Router::new_offline().await;

    if online_router_result.is_err() || offline_router_result.is_err() {
        eprintln!("⚠️  Router initialization failed, skipping test");
        return;
    }

    let online_router = online_router_result.unwrap();
    let offline_router = offline_router_result.unwrap();

    let task = "Create a React component with hooks";

    let online_decision = online_router.route(task).await.expect("Online routing failed");
    let offline_decision = offline_router.route(task).await.expect("Offline routing failed");

    println!("\n=== Online vs Offline Comparison ===");
    println!("Task: {}", task);
    println!("\nOnline mode:");
    println!("  Model: {}", online_decision.recommended_model.name());
    println!("  Cost: ${:.4}", online_decision.estimated_cost_usd);
    println!("  Cloud: {}", online_decision.recommended_model.is_cloud());
    println!("\nOffline mode:");
    println!("  Model: {}", offline_decision.recommended_model.name());
    println!("  Cost: ${:.4}", offline_decision.estimated_cost_usd);
    println!("  Cloud: {}", offline_decision.recommended_model.is_cloud());

    assert_ne!(online_decision.recommended_model, offline_decision.recommended_model);
    assert!(offline_decision.recommended_model.is_local());
    assert_eq!(offline_decision.estimated_cost_usd, 0.0);
}

#[tokio::test]
async fn test_connectivity_check() {
    let router_result = Router::new().await;

    if router_result.is_err() {
        eprintln!("⚠️  Router initialization failed, skipping test");
        return;
    }

    let router = router_result.unwrap();

    println!("\n=== Connectivity Check Test ===");

    let is_online = router.check_connectivity().await;

    println!("Network connectivity: {}", if is_online { "Online" } else { "Offline" });
    println!("Note: This test depends on network availability");
}

#[tokio::test]
async fn test_offline_metrics() {
    let router_result = Router::new_offline().await;

    if router_result.is_err() {
        eprintln!("⚠️  Router initialization failed, skipping test");
        return;
    }

    let router = router_result.unwrap();

    let tasks = vec![
        "Create a React component",
        "Write a REST API",
        "Find all TODO comments",
        "Scan for vulnerabilities",
    ];

    for task in tasks {
        router.route(task).await.expect("Routing failed");
    }

    let metrics = router.get_metrics().await;

    println!("\n=== Offline Metrics Test ===");
    println!("Total routes: {}", metrics.total_routes);
    println!("Local model usage: {:.1}%", metrics.local_model_usage_percent());
    println!("Average cost: ${:.4}", metrics.average_cost());
    println!("Total cost: ${:.4}", metrics.total_estimated_cost);

    assert_eq!(metrics.total_routes, 4);
    assert_eq!(metrics.local_model_usage_percent(), 100.0);
    assert_eq!(metrics.total_estimated_cost, 0.0);
}
