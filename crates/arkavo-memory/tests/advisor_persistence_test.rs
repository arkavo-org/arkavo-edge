#![allow(clippy::disallowed_methods)]

use arkavo_memory::{AdvisorStateStore, PersistedAdjustment};
use chrono::Utc;
use std::sync::atomic::{AtomicU64, Ordering};

async fn create_test_store() -> AdvisorStateStore {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let db_path = std::path::PathBuf::from(format!(
        "file:advisor_integ_{ts}_{id}?mode=memory&cache=shared"
    ));
    AdvisorStateStore::new(&db_path).await.unwrap()
}

fn make_adjustment(
    label: &str,
    family: &str,
    issue: &str,
    rate: f64,
    count: u32,
) -> PersistedAdjustment {
    PersistedAdjustment {
        label: label.to_string(),
        model_family: family.to_string(),
        issue: issue.to_string(),
        text: format!("Fix {issue} for {family}"),
        success_rate: rate,
        applications: count + 2,
        feedback_count: count,
        updated_at: Utc::now(),
    }
}

/// Simulate a full agent lifecycle: learn adjustments, persist validated ones,
/// "restart" with a fresh store load, and verify lessons survived.
#[tokio::test]
async fn test_cross_restart_persistence() {
    let store = create_test_store().await;

    // Session 1: agent learned three adjustments during runtime
    let validated = make_adjustment("deepseek-timeout-dynamic", "deepseek", "Timeout", 0.75, 5);
    let also_validated = make_adjustment(
        "mistral-outputloop-dynamic",
        "mistral",
        "OutputLoop",
        0.6,
        4,
    );
    let noise = make_adjustment(
        "glm-unwantedcodefence-dynamic",
        "glm",
        "UnwantedCodeFence",
        0.3,
        2,
    );

    // Only persist validated adjustments (feedback_count >= 3 AND success_rate > 0.5)
    let to_save: Vec<_> = [&validated, &also_validated, &noise]
        .iter()
        .filter(|a| a.feedback_count >= 3 && a.success_rate > 0.5)
        .cloned()
        .cloned()
        .collect();

    assert_eq!(to_save.len(), 2, "noise should be filtered out");
    store.save_batch(&to_save).await.unwrap();

    // Session 2: "restart" — load all persisted adjustments
    let loaded = store.load_all().await.unwrap();
    assert_eq!(loaded.len(), 2);

    let deepseek = loaded
        .iter()
        .find(|a| a.model_family == "deepseek")
        .unwrap();
    assert_eq!(deepseek.issue, "Timeout");
    assert!((deepseek.success_rate - 0.75).abs() < f64::EPSILON);
    assert_eq!(deepseek.feedback_count, 5);

    let mistral = loaded.iter().find(|a| a.model_family == "mistral").unwrap();
    assert_eq!(mistral.issue, "OutputLoop");
    assert_eq!(mistral.feedback_count, 4);
}

/// Verify that UPSERT correctly updates existing rows when an adjustment
/// improves over multiple sessions.
#[tokio::test]
async fn test_multi_session_improvement() {
    let store = create_test_store().await;

    // Session 1: initial save
    let adj = make_adjustment("phi-timeout-dynamic", "phi", "Timeout", 0.55, 3);
    store.save_batch(&[adj]).await.unwrap();

    // Session 2: same label, better stats after more feedback
    let improved = PersistedAdjustment {
        label: "phi-timeout-dynamic".to_string(),
        model_family: "phi".to_string(),
        issue: "Timeout".to_string(),
        text: "Be very brief.".to_string(),
        success_rate: 0.85,
        applications: 12,
        feedback_count: 10,
        updated_at: Utc::now(),
    };
    store.save_batch(&[improved]).await.unwrap();

    let loaded = store.load_all().await.unwrap();
    assert_eq!(loaded.len(), 1, "upsert should not duplicate");
    assert!((loaded[0].success_rate - 0.85).abs() < f64::EPSILON);
    assert_eq!(loaded[0].feedback_count, 10);
    assert_eq!(loaded[0].text, "Be very brief.");
}

/// Verify cleanup removes old low-performers but keeps recent and high-performers.
#[tokio::test]
async fn test_stale_cleanup_across_sessions() {
    let store = create_test_store().await;

    // Ancient low-performer (should be cleaned)
    let mut ancient_bad = make_adjustment("old-bad", "ancient", "OutputLoop", 0.4, 5);
    ancient_bad.updated_at = Utc::now() - chrono::Duration::days(60);

    // Ancient high-performer (should survive — success_rate > 0.5)
    let mut ancient_good = make_adjustment("old-good", "ancient", "Timeout", 0.9, 8);
    ancient_good.updated_at = Utc::now() - chrono::Duration::days(60);

    // Recent low-performer (should survive — too new)
    let recent_bad = make_adjustment("new-bad", "recent", "OutputLoop", 0.35, 3);

    store
        .save_batch(&[ancient_bad, ancient_good, recent_bad])
        .await
        .unwrap();

    assert_eq!(store.load_all().await.unwrap().len(), 3);

    let deleted = store.cleanup_stale(30).await.unwrap();
    assert_eq!(deleted, 1);

    let remaining = store.load_all().await.unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|a| a.label == "old-good"));
    assert!(remaining.iter().any(|a| a.label == "new-bad"));
}

/// Verify concurrent batch saves don't corrupt (SQLite WAL mode).
#[tokio::test]
async fn test_concurrent_saves() {
    let store = create_test_store().await;
    let store = std::sync::Arc::new(store);

    let mut handles = Vec::new();
    for batch in 0..5 {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            let adjustments: Vec<_> = (0..3)
                .map(|i| {
                    make_adjustment(
                        &format!("batch{batch}-adj{i}"),
                        &format!("family{batch}"),
                        "OutputLoop",
                        0.6,
                        4,
                    )
                })
                .collect();
            store.save_batch(&adjustments).await.unwrap();
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let loaded = store.load_all().await.unwrap();
    assert_eq!(loaded.len(), 15); // 5 batches × 3 adjustments
}
