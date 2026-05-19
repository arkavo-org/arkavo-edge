use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use std::path::Path;

/// A persisted TDF audit manifest record.
///
/// Captures the metadata from each cloud-bound message encryption:
/// which message was encrypted, with what algorithm, how large the
/// ciphertext was, and which policy attributes were applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub session_id: String,
    pub message_index: usize,
    pub agent_id: String,
    pub model: String,
    pub algorithm: String,
    pub ciphertext_bytes: usize,
    pub policy_attributes: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// SQLite-backed store for TDF audit manifests.
///
/// Follows the `AdvisorStateStore` pattern: own pool, PRAGMA WAL,
/// auto-init schema. Records are append-only with time-based cleanup.
pub struct TdfAuditStore {
    pool: SqlitePool,
}

impl TdfAuditStore {
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
            CREATE TABLE IF NOT EXISTS tdf_audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                message_index INTEGER NOT NULL,
                agent_id TEXT NOT NULL,
                model TEXT NOT NULL,
                algorithm TEXT NOT NULL,
                ciphertext_bytes INTEGER NOT NULL,
                policy_attributes TEXT NOT NULL,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tdf_audit_session ON tdf_audit_log (session_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_tdf_audit_created ON tdf_audit_log (created_at)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Append a batch of audit records in a single transaction.
    pub async fn save_batch(&self, records: &[AuditRecord]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for rec in records {
            let attrs_json =
                serde_json::to_string(&rec.policy_attributes).unwrap_or_else(|_| "[]".to_string());

            sqlx::query(
                r#"
                INSERT INTO tdf_audit_log
                    (session_id, message_index, agent_id, model, algorithm,
                     ciphertext_bytes, policy_attributes, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&rec.session_id)
            .bind(rec.message_index as i64)
            .bind(&rec.agent_id)
            .bind(&rec.model)
            .bind(&rec.algorithm)
            .bind(rec.ciphertext_bytes as i64)
            .bind(&attrs_json)
            .bind(rec.created_at.to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Load audit records for a specific session.
    pub async fn load_by_session(&self, session_id: &str) -> Result<Vec<AuditRecord>> {
        let rows = sqlx::query_as::<_, (String, i64, String, String, String, i64, String, String)>(
            r#"SELECT session_id, message_index, agent_id, model, algorithm,
                      ciphertext_bytes, policy_attributes, created_at
               FROM tdf_audit_log WHERE session_id = ? ORDER BY id"#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Self::row_to_record).collect())
    }

    /// Load the most recent audit records across all sessions.
    pub async fn load_recent(&self, limit: u32) -> Result<Vec<AuditRecord>> {
        let rows = sqlx::query_as::<_, (String, i64, String, String, String, i64, String, String)>(
            r#"SELECT session_id, message_index, agent_id, model, algorithm,
                      ciphertext_bytes, policy_attributes, created_at
               FROM tdf_audit_log ORDER BY id DESC LIMIT ?"#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Self::row_to_record).collect())
    }

    /// Delete audit records older than `max_age_days`.
    pub async fn cleanup_old(&self, max_age_days: i64) -> Result<u64> {
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days);

        let result = sqlx::query("DELETE FROM tdf_audit_log WHERE created_at < ?")
            .bind(cutoff.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Total number of audit records.
    pub async fn count(&self) -> Result<u64> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tdf_audit_log")
            .fetch_one(&self.pool)
            .await?;
        Ok(count as u64)
    }

    fn row_to_record(r: (String, i64, String, String, String, i64, String, String)) -> AuditRecord {
        let policy_attributes: Vec<String> = serde_json::from_str(&r.6).unwrap_or_default();

        AuditRecord {
            session_id: r.0,
            message_index: r.1 as usize,
            agent_id: r.2,
            model: r.3,
            algorithm: r.4,
            ciphertext_bytes: r.5 as usize,
            policy_attributes,
            created_at: DateTime::parse_from_rfc3339(&r.7)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    async fn create_test_store() -> TdfAuditStore {
        static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
        let test_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let db_path = std::path::PathBuf::from(format!(
            "file:arkavo_tdf_audit_test_{timestamp}_{test_id}?mode=memory&cache=shared"
        ));

        TdfAuditStore::new(&db_path).await.unwrap()
    }

    fn sample_record(session: &str, idx: usize) -> AuditRecord {
        AuditRecord {
            session_id: session.to_string(),
            message_index: idx,
            agent_id: "test-agent".to_string(),
            model: "gemini-3.5-flash".to_string(),
            algorithm: "AES-256-GCM".to_string(),
            ciphertext_bytes: 1024 + idx * 100,
            policy_attributes: vec![
                "https://arkavo.net/attr/agent-id/test-agent".to_string(),
                "https://arkavo.net/attr/role/cloud-prompt".to_string(),
            ],
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn save_and_load_by_session() {
        let store = create_test_store().await;
        let records = vec![sample_record("sess-1", 0), sample_record("sess-1", 2)];

        store.save_batch(&records).await.unwrap();

        let loaded = store.load_by_session("sess-1").await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].message_index, 0);
        assert_eq!(loaded[1].message_index, 2);
        assert_eq!(loaded[0].algorithm, "AES-256-GCM");
        assert_eq!(loaded[0].policy_attributes.len(), 2);
    }

    #[tokio::test]
    async fn load_recent_across_sessions() {
        let store = create_test_store().await;
        let records = vec![
            sample_record("sess-1", 0),
            sample_record("sess-2", 0),
            sample_record("sess-2", 1),
        ];

        store.save_batch(&records).await.unwrap();

        let recent = store.load_recent(2).await.unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[tokio::test]
    async fn load_empty() {
        let store = create_test_store().await;
        let loaded = store.load_by_session("nonexistent").await.unwrap();
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn count_records() {
        let store = create_test_store().await;
        assert_eq!(store.count().await.unwrap(), 0);

        store
            .save_batch(&[sample_record("s1", 0), sample_record("s1", 1)])
            .await
            .unwrap();
        assert_eq!(store.count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn cleanup_old_records() {
        let store = create_test_store().await;

        let mut old = sample_record("old-sess", 0);
        old.created_at = Utc::now() - chrono::Duration::days(40);

        let fresh = sample_record("fresh-sess", 0);

        store.save_batch(&[old, fresh]).await.unwrap();

        let deleted = store.cleanup_old(30).await.unwrap();
        assert_eq!(deleted, 1);

        let remaining = store.load_recent(100).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].session_id, "fresh-sess");
    }

    #[tokio::test]
    async fn batch_transaction() {
        let store = create_test_store().await;

        let records: Vec<_> = (0..10).map(|i| sample_record("batch-sess", i)).collect();
        store.save_batch(&records).await.unwrap();

        assert_eq!(store.count().await.unwrap(), 10);
    }

    #[tokio::test]
    async fn session_isolation() {
        let store = create_test_store().await;

        store
            .save_batch(&[
                sample_record("sess-a", 0),
                sample_record("sess-a", 1),
                sample_record("sess-b", 0),
            ])
            .await
            .unwrap();

        let a = store.load_by_session("sess-a").await.unwrap();
        let b = store.load_by_session("sess-b").await.unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 1);
    }
}
