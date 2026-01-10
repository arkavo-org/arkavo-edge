#![cfg(feature = "cef-ui")]

//! Integration tests for CEF UI with Architect system integration.
//!
//! These tests verify:
//! - Complexity detection triggers architect mode
//! - Plan state persistence lifecycle
//! - CEF renders two-pane architect layout
//! - Plan resume functionality

use arkavo_agui::renderer::{RendererType, create_renderer};
use arkavo_memory::{PersistedPlan, PlanStateStore, PlanStatus};
use arkavo_router::ComplexityScorer;
use chrono::Utc;
use std::time::Duration;
use tempfile::TempDir;
use uuid::Uuid;

/// Test that simple tasks do NOT trigger architect mode
#[tokio::test]
async fn test_simple_task_no_architect() {
    let scorer = ComplexityScorer::new();

    let simple_prompts = [
        "What time is it?",
        "Hello",
        "2 + 2",
        "Define recursion",
        "What is Rust?",
    ];

    for prompt in simple_prompts {
        let result = scorer.analyze(prompt);
        assert!(
            !result.architect_recommended,
            "Simple prompt '{}' should NOT trigger architect mode",
            prompt
        );
    }
}

/// Test that complex tasks with explicit multi-step patterns DO trigger architect mode
/// The scorer requires: subtasks >= 3, output_tokens >= 2000, and (keywords OR multi_step >= 2)
#[tokio::test]
async fn test_complex_task_triggers_architect() {
    let scorer = ComplexityScorer::new();

    // These prompts use explicit multi-step patterns that the scorer detects
    let complex_prompts = [
        // Has "first", "then", "after that", "finally" - triggers multi-step detection
        "Build a REST API. First, create the authentication system with JWT tokens. \
         Then, set up the database models and migrations. After that, implement \
         the API endpoints with proper validation. Finally, write comprehensive \
         integration tests for all endpoints.",
        // Has "Step 1", "Step 2", etc.
        "Create a CLI tool. Step 1: Set up argument parsing with subcommands. \
         Step 2: Implement configuration file loading. Step 3: Add logging \
         infrastructure. Step 4: Create the main command handlers. Step 5: \
         Write unit tests for each component.",
    ];

    for prompt in complex_prompts {
        let result = scorer.analyze(prompt);
        assert!(
            result.architect_recommended,
            "Complex prompt should trigger architect mode (got: recommended={}, subtasks={}, triggers={:?})",
            result.architect_recommended, result.estimated_subtasks, result.complexity_triggers
        );
        assert!(
            result.estimated_subtasks >= 3,
            "Complex prompt should estimate at least 3 subtasks (got: {})",
            result.estimated_subtasks
        );
    }
}

/// Test plan state persistence full lifecycle
#[tokio::test]
async fn test_plan_state_persistence_lifecycle() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_plans.db");

    let store = PlanStateStore::new(&db_path)
        .await
        .expect("Failed to create store");

    // 1. Create and save an executing plan
    let plan_id = Uuid::new_v4();
    let plan = PersistedPlan {
        id: plan_id,
        original_prompt: "Build a web server with authentication".to_string(),
        plan_json: r#"{"id":"test","subtasks":[]}"#.to_string(),
        status: PlanStatus::Executing,
        current_subtask: 2,
        total_subtasks: 5,
        completed_results_json: Some(r#"[{"index":0},{"index":1}]"#.to_string()),
        error_message: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.save_plan(&plan).await.expect("Failed to save plan");

    // 2. Verify plan was saved
    let retrieved = store
        .get_plan(plan_id)
        .await
        .expect("Failed to get plan")
        .expect("Plan should exist");
    assert_eq!(retrieved.original_prompt, plan.original_prompt);
    assert_eq!(retrieved.current_subtask, 2);
    assert_eq!(retrieved.total_subtasks, 5);
    assert_eq!(retrieved.status, PlanStatus::Executing);

    // 3. Simulate app crash/restart - mark executing as interrupted
    let interrupted_count = store
        .mark_executing_as_interrupted()
        .await
        .expect("Failed to mark interrupted");
    assert_eq!(interrupted_count, 1, "Should mark 1 plan as interrupted");

    // 4. Verify plan is now marked as interrupted
    let after_crash = store
        .get_plan(plan_id)
        .await
        .expect("Failed to get plan")
        .expect("Plan should exist");
    assert_eq!(after_crash.status, PlanStatus::Interrupted);

    // 5. Verify plan appears in resumable list
    let resumable = store
        .get_resumable_plans()
        .await
        .expect("Failed to get resumable");
    assert_eq!(resumable.len(), 1);
    assert_eq!(resumable[0].id, plan_id);

    // 6. Simulate resume - mark as executing again
    store
        .update_status(plan_id, PlanStatus::Executing)
        .await
        .expect("Failed to update status");

    // 7. Update progress
    store
        .update_progress(plan_id, 3, Some(r#"[{"index":0},{"index":1},{"index":2}]"#))
        .await
        .expect("Failed to update progress");

    let progressed = store
        .get_plan(plan_id)
        .await
        .expect("Failed to get plan")
        .expect("Plan should exist");
    assert_eq!(progressed.current_subtask, 3);

    // 8. Mark as completed
    store
        .mark_completed(plan_id)
        .await
        .expect("Failed to mark completed");

    let completed = store
        .get_plan(plan_id)
        .await
        .expect("Failed to get plan")
        .expect("Plan should exist");
    assert_eq!(completed.status, PlanStatus::Completed);

    // 9. Verify no longer resumable
    let resumable_after = store
        .get_resumable_plans()
        .await
        .expect("Failed to get resumable");
    assert!(
        resumable_after.is_empty(),
        "Completed plan should not be resumable"
    );
}

/// Test plan failure handling
#[tokio::test]
async fn test_plan_failure_persistence() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_plans_failure.db");

    let store = PlanStateStore::new(&db_path)
        .await
        .expect("Failed to create store");

    let plan_id = Uuid::new_v4();
    let plan = PersistedPlan {
        id: plan_id,
        original_prompt: "Failing task".to_string(),
        plan_json: "{}".to_string(),
        status: PlanStatus::Executing,
        current_subtask: 1,
        total_subtasks: 3,
        completed_results_json: None,
        error_message: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.save_plan(&plan).await.expect("Failed to save plan");

    // Mark as failed with error message
    store
        .mark_failed(plan_id, "API rate limit exceeded")
        .await
        .expect("Failed to mark failed");

    let failed = store
        .get_plan(plan_id)
        .await
        .expect("Failed to get plan")
        .expect("Plan should exist");

    assert_eq!(failed.status, PlanStatus::Failed);
    assert_eq!(
        failed.error_message,
        Some("API rate limit exceeded".to_string())
    );
}

/// Test CEF renders two-pane architect layout
#[tokio::test]
async fn test_cef_architect_two_pane_layout() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping CEF test: {}", e);
            return;
        }
    };

    // Render the architect two-pane layout
    let html = r#"
        <div style="display:flex;height:100vh;font-family:system-ui;">
            <div id="main-content" style="flex:1;padding:20px;">
                <h1>Main Content Area</h1>
                <p>Task execution results appear here</p>
            </div>
            <div id="side-panel" style="width:320px;background:#f8f9fa;padding:20px;">
                <h3>Task Progress</h3>
                <div id="subtask-list">
                    <div class="subtask">1. Planning</div>
                    <div class="subtask">2. Execution</div>
                    <div class="subtask">3. Synthesis</div>
                </div>
            </div>
        </div>
    "#;

    let render_result = renderer.render(html, "", "").await;
    assert!(render_result.is_ok(), "Should render two-pane layout");

    // Allow time for rendering
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        renderer.is_running(),
        "CEF renderer should still be running"
    );

    // Clean shutdown
    let shutdown_result = Box::new(renderer).shutdown().await;
    assert!(shutdown_result.is_ok(), "Should shutdown cleanly");
}

/// Test CEF renders planning phase indicator
#[tokio::test]
async fn test_cef_planning_phase_indicator() {
    let mut renderer = match create_renderer(RendererType::Cef).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping CEF test: {}", e);
            return;
        }
    };

    let planning_html = r#"
        <div style="padding:40px;font-family:system-ui;">
            <div style="background:#f5f5f5;padding:16px;border-radius:8px;">
                <strong>You:</strong> Build a REST API
            </div>
            <div style="background:white;padding:20px;margin-top:20px;border-radius:8px;">
                <div style="display:flex;gap:8px;align-items:center;color:#667eea;">
                    <div style="width:8px;height:8px;background:#667eea;border-radius:50%;"></div>
                    <span>Planning task decomposition...</span>
                </div>
                <div style="margin-top:12px;color:#666;">
                    Detected complex task with 5 estimated subtasks
                </div>
            </div>
        </div>
    "#;

    let render_result = renderer.render(planning_html, "", "").await;
    assert!(render_result.is_ok(), "Should render planning indicator");

    tokio::time::sleep(Duration::from_millis(300)).await;

    let shutdown_result = Box::new(renderer).shutdown().await;
    assert!(shutdown_result.is_ok());
}

/// Test recent plans retrieval
#[tokio::test]
async fn test_recent_plans_retrieval() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_recent.db");

    let store = PlanStateStore::new(&db_path)
        .await
        .expect("Failed to create store");

    // Create multiple plans
    for i in 0..5 {
        let plan = PersistedPlan {
            id: Uuid::new_v4(),
            original_prompt: format!("Task {}", i),
            plan_json: "{}".to_string(),
            status: if i % 2 == 0 {
                PlanStatus::Completed
            } else {
                PlanStatus::Failed
            },
            current_subtask: 0,
            total_subtasks: 3,
            completed_results_json: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.save_plan(&plan).await.expect("Failed to save plan");
    }

    let recent = store
        .get_recent_plans(3)
        .await
        .expect("Failed to get recent");

    assert_eq!(recent.len(), 3, "Should return exactly 3 recent plans");
}
