use crate::error::Result;
use chrono::{DateTime, Duration, Utc};
use sqlx::sqlite::SqlitePool;
use std::env;

const DEFAULT_RETENTION_HOURS: u64 = 24;
const DEFAULT_MAX_EVENTS_PER_SESSION: u64 = 100_000;
const MAX_DB_SIZE_BYTES: u64 = 1_073_741_824; // 1GB

pub struct EventStore {
    pool: SqlitePool,
    retention_hours: u64,
    max_events_per_session: u64,
}

impl EventStore {
    pub fn new(pool: SqlitePool) -> Self {
        let retention_hours = env::var("ARKAVO_EVENT_RETENTION_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_RETENTION_HOURS);

        let max_events_per_session = env::var("ARKAVO_MAX_EVENTS_PER_SESSION")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_EVENTS_PER_SESSION);

        Self {
            pool,
            retention_hours,
            max_events_per_session,
        }
    }

    pub async fn store_events(&self, events: Vec<SerializedEvent>) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for event in events {
            sqlx::query(
                r#"
                INSERT INTO events (
                    id, session_id, sequence, timestamp, agent_id, 
                    event_type, payload, schema_version
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&event.id)
            .bind(&event.session_id)
            .bind(event.sequence as i64)
            .bind(&event.timestamp)
            .bind(&event.agent_id)
            .bind(&event.event_type)
            .bind(&event.payload)
            .bind(&event.schema_version)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        // Check if we need to prune
        self.prune_if_needed().await?;

        Ok(())
    }

    pub async fn get_session_events(
        &self,
        session_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<StoredEvent>> {
        let limit = limit.unwrap_or(1000) as i64;

        let rows = sqlx::query_as::<_, StoredEvent>(
            r#"
            SELECT id, session_id, sequence, timestamp, agent_id, 
                   event_type, payload, schema_version, created_at
            FROM events
            WHERE session_id = ?
            ORDER BY sequence ASC
            LIMIT ?
            "#,
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_recent_events(
        &self,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>> {
        let query = if let Some(agent_id) = agent_id {
            sqlx::query_as::<_, StoredEvent>(
                r#"
                SELECT id, session_id, sequence, timestamp, agent_id, 
                       event_type, payload, schema_version, created_at
                FROM events
                WHERE agent_id = ?
                ORDER BY timestamp DESC
                LIMIT ?
                "#,
            )
            .bind(agent_id)
            .bind(limit as i64)
        } else {
            sqlx::query_as::<_, StoredEvent>(
                r#"
                SELECT id, session_id, sequence, timestamp, agent_id, 
                       event_type, payload, schema_version, created_at
                FROM events
                ORDER BY timestamp DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
        };

        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn get_events_since(
        &self,
        since: DateTime<Utc>,
        agent_id: Option<&str>,
    ) -> Result<Vec<StoredEvent>> {
        let query = if let Some(agent_id) = agent_id {
            sqlx::query_as::<_, StoredEvent>(
                r#"
                SELECT id, session_id, sequence, timestamp, agent_id, 
                       event_type, payload, schema_version, created_at
                FROM events
                WHERE timestamp > ? AND agent_id = ?
                ORDER BY timestamp ASC
                "#,
            )
            .bind(since.to_rfc3339())
            .bind(agent_id)
        } else {
            sqlx::query_as::<_, StoredEvent>(
                r#"
                SELECT id, session_id, sequence, timestamp, agent_id, 
                       event_type, payload, schema_version, created_at
                FROM events
                WHERE timestamp > ?
                ORDER BY timestamp ASC
                "#,
            )
            .bind(since.to_rfc3339())
        };

        Ok(query.fetch_all(&self.pool).await?)
    }

    async fn prune_if_needed(&self) -> Result<()> {
        // Check database size
        let db_size = self.get_database_size().await?;
        if db_size > MAX_DB_SIZE_BYTES {
            self.prune_oldest_events(db_size / 2).await?;
        }

        // Check session event counts
        self.prune_large_sessions().await?;

        // Remove old events based on retention
        self.prune_old_events().await?;

        Ok(())
    }

    async fn prune_old_events(&self) -> Result<()> {
        let cutoff = Utc::now() - Duration::hours(self.retention_hours as i64);

        sqlx::query(
            r#"
            DELETE FROM events
            WHERE created_at < ?
            "#,
        )
        .bind(cutoff.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn prune_large_sessions(&self) -> Result<()> {
        // Find sessions with too many events
        let large_sessions: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT session_id, COUNT(*) as event_count
            FROM events
            GROUP BY session_id
            HAVING event_count > ?
            "#,
        )
        .bind(self.max_events_per_session as i64)
        .fetch_all(&self.pool)
        .await?;

        for (session_id, count) in large_sessions {
            let to_delete = count - self.max_events_per_session as i64;

            // Delete oldest events from this session
            sqlx::query(
                r#"
                DELETE FROM events
                WHERE session_id = ? AND id IN (
                    SELECT id FROM events
                    WHERE session_id = ?
                    ORDER BY sequence ASC
                    LIMIT ?
                )
                "#,
            )
            .bind(&session_id)
            .bind(&session_id)
            .bind(to_delete)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    async fn prune_oldest_events(&self, target_size: u64) -> Result<()> {
        // Delete oldest events until we're under target size
        let current_size = self.get_database_size().await?;
        if current_size <= target_size {
            return Ok(());
        }

        // Delete in batches of 10k events
        loop {
            sqlx::query(
                r#"
                DELETE FROM events
                WHERE id IN (
                    SELECT id FROM events
                    ORDER BY created_at ASC
                    LIMIT 10000
                )
                "#,
            )
            .execute(&self.pool)
            .await?;

            let new_size = self.get_database_size().await?;
            if new_size <= target_size {
                break;
            }
        }

        // VACUUM to reclaim space
        sqlx::query("VACUUM").execute(&self.pool).await?;

        Ok(())
    }

    async fn get_database_size(&self) -> Result<u64> {
        let result: (i64,) = sqlx::query_as(
            "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(result.0 as u64)
    }

    pub async fn get_session_count(&self) -> Result<u64> {
        let result: (i64,) = sqlx::query_as("SELECT COUNT(DISTINCT session_id) FROM events")
            .fetch_one(&self.pool)
            .await?;

        Ok(result.0 as u64)
    }

    pub async fn get_total_event_count(&self) -> Result<u64> {
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
            .fetch_one(&self.pool)
            .await?;

        Ok(result.0 as u64)
    }
}

#[derive(Debug, Clone)]
pub struct SerializedEvent {
    pub id: String,
    pub session_id: String,
    pub sequence: u64,
    pub timestamp: String,
    pub agent_id: String,
    pub event_type: String,
    pub payload: Vec<u8>,
    pub schema_version: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredEvent {
    pub id: String,
    pub session_id: String,
    pub sequence: i64,
    pub timestamp: String,
    pub agent_id: String,
    pub event_type: String,
    pub payload: Vec<u8>,
    pub schema_version: String,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)] // tokio::test needs block_on internally

    #[tokio::test]
    async fn test_event_store_basic() {
        // Test will be implemented when MemoryStorage exposes test helpers
        // For now, this is a placeholder
    }
}
