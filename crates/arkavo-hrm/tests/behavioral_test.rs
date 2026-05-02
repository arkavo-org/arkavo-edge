//! Behavioral integration tests for HRM spec scenarios

use arkavo_hrm::{
    BurstResult, Conductor, ContinuationHint, InMemoryTaskStore, TaskBudget, TaskStatus, TaskStore,
};
use arkavo_test_macros::spec;
use std::time::Duration;
use uuid::Uuid;

/// Helper: create a conductor with a fresh in-memory store
fn setup() -> Conductor<InMemoryTaskStore> {
    Conductor::new(InMemoryTaskStore::new())
}

/// Helper: create a tight budget for testing enforcement
fn tight_budget(max_cost: f64, max_tokens: u64) -> TaskBudget {
    TaskBudget {
        max_cost_usd: max_cost,
        spent_usd: 0.0,
        max_tokens,
        tokens_used: 0,
        max_wall_time: Duration::from_secs(3600),
        elapsed: Duration::ZERO,
    }
}

// --- HRM-003: Budget enforcement ---

#[spec("HRM-003")]
#[tokio::test]
async fn test_budget_enforcement_cost() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), tight_budget(0.10, 100_000))
        .await
        .unwrap();

    let subtask = conductor
        .add_subtask(task.id, "Do work".into(), vec![])
        .await
        .unwrap();

    // Record a result that exceeds the $0.10 budget
    let result = BurstResult::failure(Uuid::new_v4(), "done".into()).with_metrics(
        1,
        500,
        0.15,
        Duration::from_secs(1),
    );
    conductor
        .record_result(task.id, subtask.id, result)
        .await
        .unwrap();

    // Verify budget is exhausted
    let loaded = conductor.get_task(task.id).await.unwrap();
    assert!(!loaded.budget.has_remaining());
    assert!(loaded.budget.spent_usd >= 0.15);
}

#[spec("HRM-003")]
#[tokio::test]
async fn test_budget_enforcement_tokens() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), tight_budget(10.0, 500))
        .await
        .unwrap();

    let subtask = conductor
        .add_subtask(task.id, "Do work".into(), vec![])
        .await
        .unwrap();

    // Record 600 tokens against 500 budget
    let result = BurstResult::success(Uuid::new_v4(), serde_json::json!({})).with_metrics(
        1,
        600,
        0.01,
        Duration::from_secs(1),
    );
    conductor
        .record_result(task.id, subtask.id, result)
        .await
        .unwrap();

    let loaded = conductor.get_task(task.id).await.unwrap();
    assert!(!loaded.budget.has_remaining());
    assert_eq!(loaded.budget.tokens_used, 600);
    assert_eq!(loaded.budget.remaining_tokens(), 0);
}

#[spec("HRM-003")]
#[tokio::test]
async fn test_budget_tracking_incremental() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), tight_budget(1.0, 10_000))
        .await
        .unwrap();

    // Three subtasks with incremental spending
    for (i, (cost, tokens)) in [(0.20, 1000u64), (0.30, 2000), (0.10, 500)]
        .iter()
        .enumerate()
    {
        let st = conductor
            .add_subtask(task.id, format!("Step {i}"), vec![])
            .await
            .unwrap();
        let r = BurstResult::success(Uuid::new_v4(), serde_json::json!({})).with_metrics(
            1,
            *tokens,
            *cost,
            Duration::from_secs(1),
        );
        conductor.record_result(task.id, st.id, r).await.unwrap();
    }

    let loaded = conductor.get_task(task.id).await.unwrap();
    assert!((loaded.budget.spent_usd - 0.60).abs() < f64::EPSILON);
    assert_eq!(loaded.budget.tokens_used, 3500);
    assert!(loaded.budget.has_remaining());
}

// --- HRM-005: State persistence ---

#[spec("HRM-005")]
#[tokio::test]
async fn test_state_persists_through_lifecycle() {
    let store = InMemoryTaskStore::new();
    let store_ref = std::sync::Arc::new(store);
    let conductor = Conductor::with_shared_store(store_ref.clone());

    let task = conductor
        .create_task("Persist me".into(), TaskBudget::default())
        .await
        .unwrap();

    let subtask = conductor
        .add_subtask(task.id, "Sub work".into(), vec![])
        .await
        .unwrap();

    let result = BurstResult::success(Uuid::new_v4(), serde_json::json!({"key": "value"}))
        .with_metrics(2, 300, 0.05, Duration::from_secs(5));
    conductor
        .record_result(task.id, subtask.id, result)
        .await
        .unwrap();

    // Load directly from store — simulates recovery
    let recovered = store_ref.load(task.id).await.unwrap().unwrap();
    assert_eq!(recovered.objective, "Persist me");
    assert_eq!(recovered.subtasks.len(), 1);
    assert_eq!(recovered.subtasks[0].status, TaskStatus::Completed);
    assert_eq!(recovered.budget.tokens_used, 300);
    assert!((recovered.budget.spent_usd - 0.05).abs() < f64::EPSILON);
}

#[spec("HRM-005")]
#[tokio::test]
async fn test_store_list_by_status() {
    let store = InMemoryTaskStore::new();
    let store_ref = std::sync::Arc::new(store);
    let conductor = Conductor::with_shared_store(store_ref.clone());

    let t1 = conductor
        .create_task("Task 1".into(), TaskBudget::default())
        .await
        .unwrap();
    let t2 = conductor
        .create_task("Task 2".into(), TaskBudget::default())
        .await
        .unwrap();
    let _t3 = conductor
        .create_task("Task 3".into(), TaskBudget::default())
        .await
        .unwrap();

    // Add subtasks to t1 and t2 so they become Running
    let s1 = conductor
        .add_subtask(t1.id, "work".into(), vec![])
        .await
        .unwrap();
    conductor
        .record_result(
            t1.id,
            s1.id,
            BurstResult::success(Uuid::new_v4(), serde_json::json!({})),
        )
        .await
        .unwrap();

    let s2 = conductor
        .add_subtask(t2.id, "work".into(), vec![])
        .await
        .unwrap();
    conductor
        .record_result(
            t2.id,
            s2.id,
            BurstResult::failure(Uuid::new_v4(), "err".into()),
        )
        .await
        .unwrap();

    let completed = store_ref
        .list_by_status(TaskStatus::Completed)
        .await
        .unwrap();
    let pending = store_ref.list_by_status(TaskStatus::Pending).await.unwrap();

    assert_eq!(completed.len(), 1);
    assert!(!pending.is_empty());
}

// --- HRM-004: Loop detection integration ---

#[spec("HRM-004")]
#[tokio::test]
async fn test_thrashing_blocks_add_subtask() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), TaskBudget::default())
        .await
        .unwrap();

    // Fail the same description twice via conductor (would_thrash triggers at count >= 2)
    for _ in 0..2 {
        let st = conductor
            .add_subtask(task.id, "Fix the auth bug".into(), vec![])
            .await
            .unwrap();
        conductor
            .record_result(
                task.id,
                st.id,
                BurstResult::failure(Uuid::new_v4(), "failed".into()),
            )
            .await
            .unwrap();
    }

    // 3rd attempt should be blocked by thrashing detection (would_thrash checks count >= MAX-1)
    let result = conductor
        .add_subtask(task.id, "Fix the auth bug".into(), vec![])
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        format!("{err:?}").contains("StrategicThrashing"),
        "Expected StrategicThrashing error, got: {err:?}"
    );
}

#[spec("HRM-004")]
#[tokio::test]
async fn test_thrashing_with_similar_descriptions() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), TaskBudget::default())
        .await
        .unwrap();

    // Fail exact description twice to set up near-thrashing state
    for _ in 0..2 {
        let st = conductor
            .add_subtask(
                task.id,
                "Fix the authentication bug in login".into(),
                vec![],
            )
            .await
            .unwrap();
        conductor
            .record_result(
                task.id,
                st.id,
                BurstResult::failure(Uuid::new_v4(), "failed".into()),
            )
            .await
            .unwrap();
    }

    // A similar (not identical) description should also be blocked
    let result = conductor
        .add_subtask(
            task.id,
            "Fix the authentication bug in login flow".into(),
            vec![],
        )
        .await;
    // Similar description triggers would_thrash via similarity matching
    assert!(
        result.is_err(),
        "Expected similar description to be blocked by thrashing detection"
    );
}

// --- HRM-006: Continuation hints ---

#[spec("HRM-006")]
#[tokio::test]
async fn test_continuation_hint_preserved() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), TaskBudget::default())
        .await
        .unwrap();

    let subtask = conductor
        .add_subtask(task.id, "Almost done work".into(), vec![])
        .await
        .unwrap();

    let hint = ContinuationHint::new(2, 0.9, 0.8, "Need 2 more steps");
    let result = BurstResult::partial(Uuid::new_v4(), hint).with_metrics(
        3,
        800,
        0.05,
        Duration::from_secs(10),
    );
    conductor
        .record_result(task.id, subtask.id, result)
        .await
        .unwrap();

    // Verify continuation state is accessible
    let loaded = conductor.get_task(task.id).await.unwrap();
    let st = &loaded.subtasks[0];
    // Subtask should NOT be completed (partial result)
    assert_ne!(st.status, TaskStatus::Completed);
    // Result should be stored
    assert!(st.result.is_some());
    // Budget should reflect the spending
    assert_eq!(loaded.budget.tokens_used, 800);
}

// --- HRM-002: MaxSubtasksExceeded error includes correct counts ---

#[spec("HRM-002")]
#[tokio::test]
async fn test_max_subtasks_error_reports_counts() {
    let conductor = setup();
    let mut task = conductor
        .create_task("Test".into(), TaskBudget::default())
        .await
        .unwrap();
    task.max_total_subtasks = 2;
    conductor.store().save(&task).await.unwrap();

    conductor
        .add_subtask(task.id, "Step 1".into(), vec![])
        .await
        .unwrap();
    conductor
        .add_subtask(task.id, "Step 2".into(), vec![])
        .await
        .unwrap();

    let err = conductor
        .add_subtask(task.id, "Step 3".into(), vec![])
        .await
        .unwrap_err();
    // Verify the error carries correct counts (not just that it's the right variant)
    let msg = format!("{err}");
    assert!(
        msg.contains("2/2"),
        "Error should report current=2, max=2, got: {msg}"
    );
    assert!(err.is_terminal(), "MaxSubtasksExceeded should be terminal");
}

// --- HRM-003: Budget tracking and auto-completion ---

#[spec("HRM-003")]
#[tokio::test]
async fn test_budget_exhaustion_after_spending() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), tight_budget(0.05, 100))
        .await
        .unwrap();
    // Starts with budget remaining
    assert!(
        task.budget.has_remaining(),
        "Fresh budget should have remaining"
    );

    let subtask = conductor
        .add_subtask(task.id, "Work".into(), vec![])
        .await
        .unwrap();
    // Spend MORE than budget allows
    let result = BurstResult::success(Uuid::new_v4(), serde_json::json!({})).with_metrics(
        1,
        200,
        0.10,
        Duration::from_secs(1),
    );
    conductor
        .record_result(task.id, subtask.id, result)
        .await
        .unwrap();

    let loaded = conductor.get_task(task.id).await.unwrap();
    assert!(
        !loaded.budget.has_remaining(),
        "Budget should be exhausted after overspend"
    );
    assert!(loaded.budget.spent_usd >= 0.10);
    assert_eq!(loaded.budget.tokens_used, 200);
}

#[spec("HRM-003")]
#[tokio::test]
async fn test_task_status_auto_completes() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), TaskBudget::default())
        .await
        .unwrap();

    let s1 = conductor
        .add_subtask(task.id, "Step 1".into(), vec![])
        .await
        .unwrap();
    let s2 = conductor
        .add_subtask(task.id, "Step 2".into(), vec![])
        .await
        .unwrap();

    for sid in [s1.id, s2.id] {
        conductor
            .record_result(
                task.id,
                sid,
                BurstResult::success(Uuid::new_v4(), serde_json::json!({})),
            )
            .await
            .unwrap();
    }

    let loaded = conductor.get_task(task.id).await.unwrap();
    assert_eq!(loaded.status, TaskStatus::Completed);
}

#[spec("HRM-003")]
#[tokio::test]
async fn test_task_status_fails_when_all_terminal() {
    let conductor = setup();
    let task = conductor
        .create_task("Test".into(), TaskBudget::default())
        .await
        .unwrap();

    let s1 = conductor
        .add_subtask(task.id, "A".into(), vec![])
        .await
        .unwrap();
    let s2 = conductor
        .add_subtask(task.id, "B".into(), vec![])
        .await
        .unwrap();

    // Exhaust retries: default max_retries=3, so fail 4 times each
    for sid in [s1.id, s2.id] {
        for _ in 0..4 {
            conductor
                .record_result(
                    task.id,
                    sid,
                    BurstResult::failure(Uuid::new_v4(), "err".into()),
                )
                .await
                .unwrap();
        }
    }

    let loaded = conductor.get_task(task.id).await.unwrap();
    assert_eq!(loaded.status, TaskStatus::Failed);
}
