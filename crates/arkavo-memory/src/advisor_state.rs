use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use std::path::Path;

/// A persisted prompt advisor adjustment that survives process restarts.
///
/// Only adjustments with sufficient feedback and a positive success rate
/// are persisted — noise is filtered at the save site, not here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedAdjustment {
    pub label: String,
    pub model_family: String,
    pub issue: String,
    pub text: String,
    pub success_rate: f64,
    pub applications: u32,
    pub feedback_count: u32,
    pub updated_at: DateTime<Utc>,
}

/// SQLite-backed store for prompt advisor adjustments.
///
/// Follows the `PlanStateStore` pattern: own pool, PRAGMA WAL, auto-init schema.
pub struct AdvisorStateStore {
    pool: SqlitePool,
}

impl AdvisorStateStore {
    /// Create a new store at the given database path.
    pub async fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let db_path_str = db_path.display().to_string();
        let db_url = if db_path_str.starts_with("file:") {
            format!("sqlite://{db_path_str}")
        } else {
            format!("sqlite://{db_path_str}?mode=rwc")
        };

        let pool = SqlitePool::connect(&db_url).await?;

        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA busy_timeout = 5000")
            .execute(&pool)
            .await?;

        let store = Self { pool };
        store.init_schema().await?;
        Ok(store)
    }

    async fn init_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS advisor_adjustments (
                label TEXT PRIMARY KEY,
                model_family TEXT NOT NULL,
                issue TEXT NOT NULL,
                text TEXT NOT NULL,
                success_rate REAL NOT NULL,
                applications INTEGER NOT NULL DEFAULT 0,
                feedback_count INTEGER NOT NULL DEFAULT 0,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Upsert a batch of adjustments in a single transaction.
    pub async fn save_batch(&self, adjustments: &[PersistedAdjustment]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for adj in adjustments {
            sqlx::query(
                r#"
                INSERT INTO advisor_adjustments
                    (label, model_family, issue, text, success_rate, applications, feedback_count, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(label) DO UPDATE SET
                    model_family = excluded.model_family,
                    issue = excluded.issue,
                    text = excluded.text,
                    success_rate = excluded.success_rate,
                    applications = excluded.applications,
                    feedback_count = excluded.feedback_count,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(&adj.label)
            .bind(&adj.model_family)
            .bind(&adj.issue)
            .bind(&adj.text)
            .bind(adj.success_rate)
            .bind(adj.applications as i64)
            .bind(adj.feedback_count as i64)
            .bind(adj.updated_at.to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Load all persisted adjustments.
    pub async fn load_all(&self) -> Result<Vec<PersistedAdjustment>> {
        let rows = sqlx::query_as::<_, (String, String, String, String, f64, i64, i64, String)>(
            "SELECT label, model_family, issue, text, success_rate, applications, feedback_count, updated_at FROM advisor_adjustments",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| PersistedAdjustment {
                label: r.0,
                model_family: r.1,
                issue: r.2,
                text: r.3,
                success_rate: r.4,
                applications: r.5 as u32,
                feedback_count: r.6 as u32,
                updated_at: DateTime::parse_from_rfc3339(&r.7)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
            .collect())
    }

    /// Remove a single adjustment by label.
    pub async fn delete_by_label(&self, label: &str) -> Result<()> {
        sqlx::query("DELETE FROM advisor_adjustments WHERE label = ?")
            .bind(label)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete adjustments not updated within `max_age_days` that have low success rates.
    pub async fn cleanup_stale(&self, max_age_days: i64) -> Result<u64> {
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days);

        let result = sqlx::query(
            "DELETE FROM advisor_adjustments WHERE updated_at < ? AND success_rate <= 0.5",
        )
        .bind(cutoff.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    async fn create_test_store() -> AdvisorStateStore {
        static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
        let test_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let db_path = std::path::PathBuf::from(format!(
            "file:arkavo_advisor_state_test_{timestamp}_{test_id}?mode=memory&cache=shared"
        ));

        AdvisorStateStore::new(&db_path).await.unwrap()
    }

    fn sample_adjustment(label: &str, family: &str, issue: &str) -> PersistedAdjustment {
        PersistedAdjustment {
            label: label.to_string(),
            model_family: family.to_string(),
            issue: issue.to_string(),
            text: format!("Advice for {issue}"),
            success_rate: 0.7,
            applications: 5,
            feedback_count: 4,
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_save_and_load() {
        let store = create_test_store().await;
        let adj = sample_adjustment("glm-loop-dyn", "glm", "OutputLoop");

        store.save_batch(&[adj.clone()]).await.unwrap();

        let loaded = store.load_all().await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].label, "glm-loop-dyn");
        assert_eq!(loaded[0].model_family, "glm");
        assert_eq!(loaded[0].issue, "OutputLoop");
        assert!((loaded[0].success_rate - 0.7).abs() < f64::EPSILON);
        assert_eq!(loaded[0].applications, 5);
        assert_eq!(loaded[0].feedback_count, 4);
    }

    #[tokio::test]
    async fn test_upsert_updates() {
        let store = create_test_store().await;
        let mut adj = sample_adjustment("deepseek-fence-dyn", "deepseek", "UnwantedCodeFence");

        store.save_batch(&[adj.clone()]).await.unwrap();

        adj.success_rate = 0.9;
        adj.feedback_count = 10;
        store.save_batch(&[adj]).await.unwrap();

        let loaded = store.load_all().await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert!((loaded[0].success_rate - 0.9).abs() < f64::EPSILON);
        assert_eq!(loaded[0].feedback_count, 10);
    }

    #[tokio::test]
    async fn test_load_empty() {
        let store = create_test_store().await;
        let loaded = store.load_all().await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_delete_by_label() {
        let store = create_test_store().await;
        let a = sample_adjustment("a-label", "glm", "OutputLoop");
        let b = sample_adjustment("b-label", "deepseek", "Timeout");

        store.save_batch(&[a, b]).await.unwrap();
        assert_eq!(store.load_all().await.unwrap().len(), 2);

        store.delete_by_label("a-label").await.unwrap();
        let remaining = store.load_all().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].label, "b-label");
    }

    #[tokio::test]
    async fn test_cleanup_stale() {
        let store = create_test_store().await;

        let mut old = sample_adjustment("old-adj", "glm", "OutputLoop");
        old.success_rate = 0.4; // low performer
        old.updated_at = Utc::now() - chrono::Duration::days(40);

        let mut fresh = sample_adjustment("fresh-adj", "deepseek", "Timeout");
        fresh.success_rate = 0.4;
        fresh.updated_at = Utc::now(); // recent

        let mut old_good = sample_adjustment("old-good", "gemma", "OutputLoop");
        old_good.success_rate = 0.8; // high performer
        old_good.updated_at = Utc::now() - chrono::Duration::days(40);

        store.save_batch(&[old, fresh, old_good]).await.unwrap();

        let deleted = store.cleanup_stale(30).await.unwrap();
        assert_eq!(deleted, 1); // only old low-performer

        let remaining = store.load_all().await.unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().any(|a| a.label == "fresh-adj"));
        assert!(remaining.iter().any(|a| a.label == "old-good"));
    }

    #[tokio::test]
    async fn test_save_batch_transaction() {
        let store = create_test_store().await;

        let adjustments: Vec<_> = (0..5)
            .map(|i| sample_adjustment(&format!("adj-{i}"), "test", "OutputLoop"))
            .collect();

        store.save_batch(&adjustments).await.unwrap();

        let loaded = store.load_all().await.unwrap();
        assert_eq!(loaded.len(), 5);
    }
}
