//! SQLite-backed persistence for trust events and score cache.
//!
//! Follows the arkavo-memory store pattern: own pool, PRAGMA WAL,
//! auto-init schema. Trust events are append-only; scores are cached
//! with TTL-based invalidation.

use chrono::{DateTime, Utc};
use sqlx::sqlite::SqlitePool;
use std::path::Path;

use crate::types::{TrustEvent, TrustScore};

/// SQLite-backed trust data store.
pub struct TrustStore {
    pool: SqlitePool,
}

impl TrustStore {
    /// Create a new store at the given database path.
    pub async fn new(db_path: &Path) -> Result<Self, TrustStoreError> {
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

    /// Create an in-memory store for testing.
    pub async fn new_in_memory() -> Result<Self, TrustStoreError> {
        let pool = SqlitePool::connect("sqlite::memory:").await?;
        let store = Self { pool };
        store.init_schema().await?;
        Ok(store)
    }

    async fn init_schema(&self) -> Result<(), TrustStoreError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS trust_events (
                event_id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                subject_id TEXT NOT NULL,
                issuer_id TEXT NOT NULL,
                timestamp DATETIME NOT NULL,
                payload_json TEXT NOT NULL,
                dimensions_affected TEXT,
                signature_json TEXT,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS trust_score_cache (
                subject_id TEXT NOT NULL,
                domain TEXT NOT NULL,
                score_json TEXT NOT NULL,
                computed_at DATETIME NOT NULL,
                expires_at DATETIME NOT NULL,
                PRIMARY KEY (subject_id, domain)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_trust_events_subject \
             ON trust_events (subject_id, timestamp DESC)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_trust_events_type \
             ON trust_events (event_type)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Store a trust event. Returns false if duplicate (idempotent).
    pub async fn store_event(&self, event: &TrustEvent) -> Result<bool, TrustStoreError> {
        let payload_json = serde_json::to_string(&event.payload)?;
        let dims_json = event
            .dimensions_affected
            .as_ref()
            .map(|d| serde_json::to_string(d).unwrap_or_default());
        let sig_json = event
            .signature
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default());

        let result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO trust_events
                (event_id, event_type, subject_id, issuer_id, timestamp,
                 payload_json, dimensions_affected, signature_json)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&event.event_id)
        .bind(&event.event_type)
        .bind(&event.subject_id)
        .bind(&event.issuer_id)
        .bind(event.timestamp)
        .bind(&payload_json)
        .bind(&dims_json)
        .bind(&sig_json)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Check if an event already exists.
    pub async fn event_exists(&self, event_id: &str) -> Result<bool, TrustStoreError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM trust_events WHERE event_id = ?")
            .bind(event_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0 > 0)
    }

    /// Query events for a subject with optional filters.
    pub async fn query_events(
        &self,
        subject_id: &str,
        event_types: Option<&[String]>,
        after: Option<DateTime<Utc>>,
        before: Option<DateTime<Utc>>,
        limit: u32,
    ) -> Result<(Vec<TrustEvent>, u64), TrustStoreError> {
        // Count total matching
        let count_row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM trust_events WHERE subject_id = ?")
                .bind(subject_id)
                .fetch_one(&self.pool)
                .await?;
        let total_count = count_row.0 as u64;

        // Build filtered query
        let mut query = String::from(
            "SELECT event_id, event_type, subject_id, issuer_id, timestamp, \
             payload_json, dimensions_affected, signature_json \
             FROM trust_events WHERE subject_id = ?",
        );
        let mut bind_values: Vec<String> = vec![subject_id.to_string()];

        if let Some(types) = event_types {
            if !types.is_empty() {
                let placeholders: Vec<&str> = types.iter().map(|_| "?").collect();
                query.push_str(&format!(" AND event_type IN ({})", placeholders.join(",")));
                for t in types {
                    bind_values.push(t.clone());
                }
            }
        }

        if let Some(after_ts) = after {
            query.push_str(" AND timestamp > ?");
            bind_values.push(after_ts.to_rfc3339());
        }
        if let Some(before_ts) = before {
            query.push_str(" AND timestamp < ?");
            bind_values.push(before_ts.to_rfc3339());
        }

        query.push_str(" ORDER BY timestamp DESC LIMIT ?");
        bind_values.push(limit.to_string());

        let mut sql_query = sqlx::query_as::<_, EventRow>(&query);
        for val in &bind_values {
            sql_query = sql_query.bind(val);
        }

        let rows: Vec<EventRow> = sql_query.fetch_all(&self.pool).await?;

        let events = rows
            .into_iter()
            .map(|row| row.into_trust_event())
            .collect::<Result<Vec<_>, _>>()?;

        Ok((events, total_count))
    }

    /// Cache a computed trust score.
    pub async fn cache_score(&self, score: &TrustScore) -> Result<(), TrustStoreError> {
        let score_json = serde_json::to_string(score)?;
        let domain = score.domain.as_deref().unwrap_or("general");

        sqlx::query(
            r#"
            INSERT INTO trust_score_cache (subject_id, domain, score_json, computed_at, expires_at)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(subject_id, domain)
            DO UPDATE SET score_json = excluded.score_json,
                         computed_at = excluded.computed_at,
                         expires_at = excluded.expires_at
            "#,
        )
        .bind(&score.subject_id)
        .bind(domain)
        .bind(&score_json)
        .bind(score.validity.issued_at)
        .bind(score.validity.expires_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Load a cached score if still valid.
    pub async fn load_cached_score(
        &self,
        subject_id: &str,
        domain: &str,
    ) -> Result<Option<TrustScore>, TrustStoreError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT score_json FROM trust_score_cache \
             WHERE subject_id = ? AND domain = ? AND expires_at > ?",
        )
        .bind(subject_id)
        .bind(domain)
        .bind(Utc::now())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((json,)) => {
                let score: TrustScore = serde_json::from_str(&json)?;
                Ok(Some(score))
            }
            None => Ok(None),
        }
    }

    /// Invalidate cached scores for a subject.
    pub async fn invalidate_cache(&self, subject_id: &str) -> Result<(), TrustStoreError> {
        sqlx::query("DELETE FROM trust_score_cache WHERE subject_id = ?")
            .bind(subject_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Remove expired cache entries.
    pub async fn cleanup_expired(&self) -> Result<u64, TrustStoreError> {
        let result = sqlx::query("DELETE FROM trust_score_cache WHERE expires_at <= ?")
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

/// Internal row type for sqlx query_as.
#[derive(sqlx::FromRow)]
struct EventRow {
    event_id: String,
    event_type: String,
    subject_id: String,
    issuer_id: String,
    timestamp: DateTime<Utc>,
    payload_json: String,
    dimensions_affected: Option<String>,
    signature_json: Option<String>,
}

impl EventRow {
    fn into_trust_event(self) -> Result<TrustEvent, TrustStoreError> {
        let payload: serde_json::Value = serde_json::from_str(&self.payload_json)?;
        let dimensions_affected: Option<Vec<String>> = self
            .dimensions_affected
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?;
        let signature = self
            .signature_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?;

        Ok(TrustEvent {
            event_id: self.event_id,
            event_type: self.event_type,
            subject_id: self.subject_id,
            issuer_id: self.issuer_id,
            timestamp: self.timestamp,
            payload,
            dimensions_affected,
            signature,
            co_signatures: None,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TrustStoreError {
    #[error("Database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    async fn test_store() -> TrustStore {
        TrustStore::new_in_memory().await.unwrap()
    }

    fn test_event(id: &str, subject: &str, event_type: &str) -> TrustEvent {
        TrustEvent {
            event_id: id.to_string(),
            event_type: event_type.to_string(),
            subject_id: subject.to_string(),
            issuer_id: "did:key:z6MkIssuer".to_string(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"outcome": "success"}),
            dimensions_affected: Some(vec!["performance".to_string()]),
            signature: None,
            co_signatures: None,
        }
    }

    #[tokio::test]
    async fn store_and_query_events() {
        let store = test_store().await;
        let subject = "did:key:z6MkAgent1";

        store
            .store_event(&test_event("evt_1", subject, "contract.completed"))
            .await
            .unwrap();
        store
            .store_event(&test_event("evt_2", subject, "contract.failed"))
            .await
            .unwrap();
        store
            .store_event(&test_event(
                "evt_3",
                "did:key:z6MkOther",
                "contract.completed",
            ))
            .await
            .unwrap();

        let (events, total) = store
            .query_events(subject, None, None, None, 50)
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn duplicate_event_is_idempotent() {
        let store = test_store().await;
        let event = test_event("evt_dup", "did:key:z6MkAgent1", "contract.completed");

        let inserted = store.store_event(&event).await.unwrap();
        assert!(inserted);

        let inserted2 = store.store_event(&event).await.unwrap();
        assert!(!inserted2);
    }

    #[tokio::test]
    async fn event_exists_check() {
        let store = test_store().await;
        assert!(!store.event_exists("evt_missing").await.unwrap());

        store
            .store_event(&test_event(
                "evt_check",
                "did:key:z6MkAgent1",
                "contract.completed",
            ))
            .await
            .unwrap();
        assert!(store.event_exists("evt_check").await.unwrap());
    }

    #[tokio::test]
    async fn filter_by_event_type() {
        let store = test_store().await;
        let subject = "did:key:z6MkAgent1";

        store
            .store_event(&test_event("evt_a", subject, "contract.completed"))
            .await
            .unwrap();
        store
            .store_event(&test_event("evt_b", subject, "security.incident"))
            .await
            .unwrap();

        let types = vec!["contract.completed".to_string()];
        let (events, _) = store
            .query_events(subject, Some(&types), None, None, 50)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "contract.completed");
    }

    #[tokio::test]
    async fn cache_and_load_score() {
        let store = test_store().await;
        let now = Utc::now();

        let score = TrustScore {
            schema_version: SCHEMA_VERSION.to_string(),
            subject_id: "did:key:z6MkAgent1".to_string(),
            provider_id: "did:key:z6MkProv".to_string(),
            score: TrustScoreValue {
                composite: 750,
                dimensions: std::collections::HashMap::new(),
            },
            domain: Some("code-execution".to_string()),
            domain_match: Some(true),
            validity: ValidityWindow {
                issued_at: now,
                expires_at: now + chrono::Duration::hours(1),
                max_age_seconds: 3600,
            },
            metadata: None,
            authorized: Some(true),
            signature: None,
        };

        store.cache_score(&score).await.unwrap();

        let loaded = store
            .load_cached_score("did:key:z6MkAgent1", "code-execution")
            .await
            .unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().score.composite, 750);
    }

    #[tokio::test]
    async fn expired_cache_not_returned() {
        let store = test_store().await;
        let now = Utc::now();

        let score = TrustScore {
            schema_version: SCHEMA_VERSION.to_string(),
            subject_id: "did:key:z6MkAgent1".to_string(),
            provider_id: "did:key:z6MkProv".to_string(),
            score: TrustScoreValue {
                composite: 500,
                dimensions: std::collections::HashMap::new(),
            },
            domain: Some("general".to_string()),
            domain_match: None,
            validity: ValidityWindow {
                issued_at: now - chrono::Duration::hours(2),
                expires_at: now - chrono::Duration::hours(1),
                max_age_seconds: 3600,
            },
            metadata: None,
            authorized: None,
            signature: None,
        };

        store.cache_score(&score).await.unwrap();

        let loaded = store
            .load_cached_score("did:key:z6MkAgent1", "general")
            .await
            .unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn invalidate_cache_removes_entries() {
        let store = test_store().await;
        let now = Utc::now();

        let score = TrustScore {
            schema_version: SCHEMA_VERSION.to_string(),
            subject_id: "did:key:z6MkAgent1".to_string(),
            provider_id: "did:key:z6MkProv".to_string(),
            score: TrustScoreValue {
                composite: 800,
                dimensions: std::collections::HashMap::new(),
            },
            domain: Some("general".to_string()),
            domain_match: None,
            validity: ValidityWindow {
                issued_at: now,
                expires_at: now + chrono::Duration::hours(1),
                max_age_seconds: 3600,
            },
            metadata: None,
            authorized: None,
            signature: None,
        };

        store.cache_score(&score).await.unwrap();
        store.invalidate_cache("did:key:z6MkAgent1").await.unwrap();

        let loaded = store
            .load_cached_score("did:key:z6MkAgent1", "general")
            .await
            .unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn cleanup_expired_entries() {
        let store = test_store().await;
        let now = Utc::now();

        // Insert an expired entry
        let expired = TrustScore {
            schema_version: SCHEMA_VERSION.to_string(),
            subject_id: "did:key:z6MkExpired".to_string(),
            provider_id: "did:key:z6MkProv".to_string(),
            score: TrustScoreValue {
                composite: 100,
                dimensions: std::collections::HashMap::new(),
            },
            domain: Some("general".to_string()),
            domain_match: None,
            validity: ValidityWindow {
                issued_at: now - chrono::Duration::hours(2),
                expires_at: now - chrono::Duration::hours(1),
                max_age_seconds: 3600,
            },
            metadata: None,
            authorized: None,
            signature: None,
        };

        store.cache_score(&expired).await.unwrap();
        let removed = store.cleanup_expired().await.unwrap();
        assert_eq!(removed, 1);
    }
}
