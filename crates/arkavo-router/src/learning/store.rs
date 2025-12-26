//! SQLite-backed persistence for learning data
//!
//! This module provides persistent storage for episodic memory, lessons,
//! and agent utilities using SQLite.

#[cfg(feature = "persistence")]
mod inner {
    use super::super::{AgentUtility, Episode, Lesson};
    use crate::learning::coordination::CoordinationMetrics;
    use sqlx::Row;
    use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
    use std::path::Path;
    use thiserror::Error;
    use uuid::Uuid;

    /// Error type for learning store operations
    #[derive(Debug, Error)]
    pub enum StoreError {
        #[error("Database error: {0}")]
        Database(#[from] sqlx::Error),
        #[error("Serialization error: {0}")]
        Serialization(#[from] serde_json::Error),
        #[error("Not found")]
        NotFound,
    }

    type Result<T> = std::result::Result<T, StoreError>;

    /// SQLite-backed learning store
    pub struct LearningStore {
        pool: SqlitePool,
    }

    impl LearningStore {
        /// Create a new learning store at the given path
        pub async fn new(db_path: &Path) -> Result<Self> {
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }

            let abs_path = std::fs::canonicalize(db_path).unwrap_or_else(|_| db_path.to_path_buf());
            let database_url = format!("sqlite:{}?mode=rwc", abs_path.display());

            let pool = SqlitePoolOptions::new()
                .max_connections(5)
                .connect(&database_url)
                .await?;

            let store = Self { pool };
            store.ensure_tables().await?;
            Ok(store)
        }

        async fn ensure_tables(&self) -> Result<()> {
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS episodes (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    swarm_id TEXT NOT NULL,
                    task_id TEXT NOT NULL,
                    task_category TEXT NOT NULL,
                    observation_json TEXT NOT NULL,
                    outcome_json TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    context_hash BLOB NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                )
                "#,
            )
            .execute(&self.pool)
            .await?;

            sqlx::query(r#"CREATE INDEX IF NOT EXISTS idx_episodes_agent ON episodes(agent_id)"#)
                .execute(&self.pool)
                .await?;

            sqlx::query(r#"CREATE INDEX IF NOT EXISTS idx_episodes_swarm ON episodes(swarm_id)"#)
                .execute(&self.pool)
                .await?;

            sqlx::query(
                r#"CREATE INDEX IF NOT EXISTS idx_episodes_category ON episodes(task_category)"#,
            )
            .execute(&self.pool)
            .await?;

            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS lessons (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    swarm_id TEXT NOT NULL,
                    category TEXT NOT NULL,
                    pattern_json TEXT NOT NULL,
                    confidence REAL NOT NULL,
                    episode_count INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    propagated INTEGER NOT NULL DEFAULT 0,
                    signature BLOB NOT NULL,
                    hash BLOB NOT NULL
                )
                "#,
            )
            .execute(&self.pool)
            .await?;

            sqlx::query(r#"CREATE INDEX IF NOT EXISTS idx_lessons_swarm ON lessons(swarm_id)"#)
                .execute(&self.pool)
                .await?;

            sqlx::query(
                r#"CREATE INDEX IF NOT EXISTS idx_lessons_propagated ON lessons(propagated)"#,
            )
            .execute(&self.pool)
            .await?;

            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS lesson_votes (
                    lesson_id TEXT NOT NULL,
                    voter_id TEXT NOT NULL,
                    approve INTEGER NOT NULL,
                    evidence_json TEXT,
                    signature BLOB NOT NULL,
                    voted_at TEXT NOT NULL,
                    PRIMARY KEY (lesson_id, voter_id)
                )
                "#,
            )
            .execute(&self.pool)
            .await?;

            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS coordination_metrics (
                    id TEXT PRIMARY KEY,
                    swarm_id TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    metrics_json TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                )
                "#,
            )
            .execute(&self.pool)
            .await?;

            sqlx::query(
                r#"CREATE INDEX IF NOT EXISTS idx_metrics_swarm ON coordination_metrics(swarm_id)"#,
            )
            .execute(&self.pool)
            .await?;

            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS agent_utilities (
                    agent_id TEXT PRIMARY KEY,
                    swarm_id TEXT NOT NULL,
                    utility_json TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                )
                "#,
            )
            .execute(&self.pool)
            .await?;

            Ok(())
        }

        /// Store an episode
        pub async fn store_episode(&self, episode: &Episode) -> Result<()> {
            let observation_json = serde_json::to_string(&episode.observation)?;
            let outcome_json = serde_json::to_string(&episode.outcome)?;

            sqlx::query(
                r#"
                INSERT INTO episodes (id, agent_id, swarm_id, task_id, task_category,
                    observation_json, outcome_json, timestamp, context_hash)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(episode.id.to_string())
            .bind(&episode.agent_id)
            .bind(&episode.swarm_id)
            .bind(episode.task_id.to_string())
            .bind(&episode.task_category)
            .bind(&observation_json)
            .bind(&outcome_json)
            .bind(episode.timestamp.to_rfc3339())
            .bind(&episode.context_hash[..])
            .execute(&self.pool)
            .await?;

            Ok(())
        }

        /// Get episodes for an agent
        #[allow(clippy::cast_possible_wrap)]
        pub async fn get_episodes(&self, agent_id: &str, limit: usize) -> Result<Vec<Episode>> {
            let rows = sqlx::query(
                r#"
                SELECT * FROM episodes
                WHERE agent_id = ?
                ORDER BY timestamp DESC
                LIMIT ?
                "#,
            )
            .bind(agent_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

            rows.iter().map(|row| self.row_to_episode(row)).collect()
        }

        /// Get episodes by category
        #[allow(clippy::cast_possible_wrap)]
        pub async fn get_episodes_by_category(
            &self,
            category: &str,
            limit: usize,
        ) -> Result<Vec<Episode>> {
            let rows = sqlx::query(
                r#"
                SELECT * FROM episodes
                WHERE task_category = ?
                ORDER BY timestamp DESC
                LIMIT ?
                "#,
            )
            .bind(category)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

            rows.iter().map(|row| self.row_to_episode(row)).collect()
        }

        fn row_to_episode(&self, row: &sqlx::sqlite::SqliteRow) -> Result<Episode> {
            let id_str: String = row.get("id");
            let task_id_str: String = row.get("task_id");
            let observation_json: String = row.get("observation_json");
            let outcome_json: String = row.get("outcome_json");
            let timestamp_str: String = row.get("timestamp");
            let context_hash_bytes: Vec<u8> = row.get("context_hash");

            let mut context_hash = [0u8; 32];
            context_hash.copy_from_slice(&context_hash_bytes[..32.min(context_hash_bytes.len())]);

            Ok(Episode {
                id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()),
                agent_id: row.get("agent_id"),
                swarm_id: row.get("swarm_id"),
                task_id: Uuid::parse_str(&task_id_str).unwrap_or_else(|_| Uuid::new_v4()),
                task_category: row.get("task_category"),
                observation: serde_json::from_str(&observation_json)?,
                outcome: serde_json::from_str(&outcome_json)?,
                timestamp: chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                context_hash,
            })
        }

        /// Store a lesson
        pub async fn store_lesson(&self, lesson: &Lesson) -> Result<()> {
            let pattern_json = serde_json::to_string(&lesson.pattern)?;
            let hash = lesson.compute_hash();

            sqlx::query(
                r#"
                INSERT OR REPLACE INTO lessons (id, agent_id, swarm_id, category, pattern_json,
                    confidence, episode_count, created_at, updated_at, propagated, signature, hash)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(lesson.id.to_string())
            .bind(&lesson.agent_id)
            .bind(&lesson.swarm_id)
            .bind(&lesson.category)
            .bind(&pattern_json)
            .bind(lesson.confidence)
            .bind(lesson.episode_count as i64)
            .bind(lesson.created_at.to_rfc3339())
            .bind(lesson.updated_at.to_rfc3339())
            .bind(i32::from(lesson.propagated))
            .bind(&lesson.signature)
            .bind(&hash[..])
            .execute(&self.pool)
            .await?;

            Ok(())
        }

        /// Get lessons for a swarm
        pub async fn get_lessons(&self, swarm_id: &str) -> Result<Vec<Lesson>> {
            let rows = sqlx::query(
                r#"
                SELECT * FROM lessons
                WHERE swarm_id = ?
                ORDER BY confidence DESC
                "#,
            )
            .bind(swarm_id)
            .fetch_all(&self.pool)
            .await?;

            rows.iter().map(|row| self.row_to_lesson(row)).collect()
        }

        /// Get unpropagated lessons
        #[allow(clippy::cast_possible_wrap)]
        pub async fn get_unpropagated_lessons(&self, limit: usize) -> Result<Vec<Lesson>> {
            let rows = sqlx::query(
                r#"
                SELECT * FROM lessons
                WHERE propagated = 0
                ORDER BY confidence DESC
                LIMIT ?
                "#,
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;

            rows.iter().map(|row| self.row_to_lesson(row)).collect()
        }

        /// Mark a lesson as propagated
        pub async fn mark_propagated(&self, lesson_id: Uuid) -> Result<()> {
            sqlx::query(
                r#"
                UPDATE lessons SET propagated = 1, updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(lesson_id.to_string())
            .execute(&self.pool)
            .await?;

            Ok(())
        }

        fn row_to_lesson(&self, row: &sqlx::sqlite::SqliteRow) -> Result<Lesson> {
            let id_str: String = row.get("id");
            let pattern_json: String = row.get("pattern_json");
            let created_at_str: String = row.get("created_at");
            let updated_at_str: String = row.get("updated_at");
            let propagated: i32 = row.get("propagated");

            Ok(Lesson {
                id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4()),
                agent_id: row.get("agent_id"),
                swarm_id: row.get("swarm_id"),
                category: row.get("category"),
                pattern: serde_json::from_str(&pattern_json)?,
                confidence: row.get("confidence"),
                episode_count: row.get::<i64, _>("episode_count") as u32,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                propagated: propagated != 0,
                signature: row.get("signature"),
            })
        }

        /// Save agent utility
        pub async fn save_agent_utility(&self, agent: &AgentUtility, swarm_id: &str) -> Result<()> {
            let utility_json = serde_json::to_string(agent)?;

            sqlx::query(
                r#"
                INSERT OR REPLACE INTO agent_utilities (agent_id, swarm_id, utility_json, updated_at)
                VALUES (?, ?, ?, ?)
                "#,
            )
            .bind(&agent.agent_id)
            .bind(swarm_id)
            .bind(&utility_json)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;

            Ok(())
        }

        /// Load agent utility
        pub async fn load_agent_utility(&self, agent_id: &str) -> Result<Option<AgentUtility>> {
            let row = sqlx::query(
                r#"
                SELECT utility_json FROM agent_utilities
                WHERE agent_id = ?
                "#,
            )
            .bind(agent_id)
            .fetch_optional(&self.pool)
            .await?;

            match row {
                Some(row) => {
                    let utility_json: String = row.get("utility_json");
                    Ok(Some(serde_json::from_str(&utility_json)?))
                }
                None => Ok(None),
            }
        }

        /// Store coordination metrics
        pub async fn store_metrics(&self, metrics: &CoordinationMetrics) -> Result<()> {
            let metrics_json = serde_json::to_string(metrics)?;

            sqlx::query(
                r#"
                INSERT INTO coordination_metrics (id, swarm_id, timestamp, metrics_json)
                VALUES (?, ?, ?, ?)
                "#,
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&metrics.swarm_id)
            .bind(metrics.timestamp.to_rfc3339())
            .bind(&metrics_json)
            .execute(&self.pool)
            .await?;

            Ok(())
        }

        /// Get recent metrics for a swarm
        pub async fn get_recent_metrics(
            &self,
            swarm_id: &str,
            hours: u32,
        ) -> Result<Vec<CoordinationMetrics>> {
            let since = chrono::Utc::now() - chrono::Duration::hours(hours as i64);

            let rows = sqlx::query(
                r#"
                SELECT metrics_json FROM coordination_metrics
                WHERE swarm_id = ? AND timestamp >= ?
                ORDER BY timestamp DESC
                "#,
            )
            .bind(swarm_id)
            .bind(since.to_rfc3339())
            .fetch_all(&self.pool)
            .await?;

            rows.iter()
                .map(|row| {
                    let json: String = row.get("metrics_json");
                    serde_json::from_str(&json).map_err(StoreError::from)
                })
                .collect()
        }
    }
}

#[cfg(feature = "persistence")]
pub use inner::{LearningStore, StoreError};

#[cfg(not(feature = "persistence"))]
mod stub {
    use super::super::{AgentUtility, Episode, Lesson};
    use crate::learning::coordination::CoordinationMetrics;
    use std::path::Path;
    use thiserror::Error;
    use uuid::Uuid;

    #[derive(Debug, Error)]
    pub enum StoreError {
        #[error("Persistence feature not enabled")]
        NotEnabled,
    }

    type Result<T> = std::result::Result<T, StoreError>;

    /// Stub store when persistence is disabled
    pub struct LearningStore;

    impl LearningStore {
        pub async fn new(_db_path: &Path) -> Result<Self> {
            Err(StoreError::NotEnabled)
        }

        pub async fn store_episode(&self, _episode: &Episode) -> Result<()> {
            Err(StoreError::NotEnabled)
        }

        pub async fn get_episodes(&self, _agent_id: &str, _limit: usize) -> Result<Vec<Episode>> {
            Err(StoreError::NotEnabled)
        }

        pub async fn store_lesson(&self, _lesson: &Lesson) -> Result<()> {
            Err(StoreError::NotEnabled)
        }

        pub async fn get_lessons(&self, _swarm_id: &str) -> Result<Vec<Lesson>> {
            Err(StoreError::NotEnabled)
        }

        pub async fn get_unpropagated_lessons(&self, _limit: usize) -> Result<Vec<Lesson>> {
            Err(StoreError::NotEnabled)
        }

        pub async fn mark_propagated(&self, _lesson_id: Uuid) -> Result<()> {
            Err(StoreError::NotEnabled)
        }

        pub async fn save_agent_utility(
            &self,
            _agent: &AgentUtility,
            _swarm_id: &str,
        ) -> Result<()> {
            Err(StoreError::NotEnabled)
        }

        pub async fn load_agent_utility(&self, _agent_id: &str) -> Result<Option<AgentUtility>> {
            Err(StoreError::NotEnabled)
        }

        pub async fn store_metrics(&self, _metrics: &CoordinationMetrics) -> Result<()> {
            Err(StoreError::NotEnabled)
        }
    }
}

#[cfg(not(feature = "persistence"))]
pub use stub::{LearningStore, StoreError};
