use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use std::path::Path;
use uuid::Uuid;

const DEFAULT_RETENTION_DAYS: i64 = 7;

/// Status of an architect plan
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanStatus {
    /// Plan is being created
    Planning,
    /// Plan is being executed
    Executing,
    /// Plan completed successfully
    Completed,
    /// Plan was interrupted (app closed, error, etc.)
    Interrupted,
    /// Plan failed
    Failed,
}

impl std::fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanStatus::Planning => write!(f, "planning"),
            PlanStatus::Executing => write!(f, "executing"),
            PlanStatus::Completed => write!(f, "completed"),
            PlanStatus::Interrupted => write!(f, "interrupted"),
            PlanStatus::Failed => write!(f, "failed"),
        }
    }
}

impl From<&str> for PlanStatus {
    fn from(s: &str) -> Self {
        match s {
            "planning" => PlanStatus::Planning,
            "executing" => PlanStatus::Executing,
            "completed" => PlanStatus::Completed,
            "interrupted" => PlanStatus::Interrupted,
            "failed" => PlanStatus::Failed,
            _ => PlanStatus::Interrupted,
        }
    }
}

/// Persisted state of an architect plan for resume capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedPlan {
    /// Unique plan ID
    pub id: Uuid,
    /// Original user prompt
    pub original_prompt: String,
    /// Serialized ArchitectPlan JSON
    pub plan_json: String,
    /// Current status
    pub status: PlanStatus,
    /// Index of current/next subtask to execute (0-indexed)
    pub current_subtask: usize,
    /// Total number of subtasks
    pub total_subtasks: usize,
    /// Serialized results from completed subtasks
    pub completed_results_json: Option<String>,
    /// Error message if failed
    pub error_message: Option<String>,
    /// When the plan was created
    pub created_at: DateTime<Utc>,
    /// When the plan was last updated
    pub updated_at: DateTime<Utc>,
}

/// Store for persisting architect plan state across app restarts
pub struct PlanStateStore {
    pool: SqlitePool,
    retention_days: i64,
}

impl PlanStateStore {
    /// Create a new plan state store at the given database path
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

        let store = Self {
            pool,
            retention_days: DEFAULT_RETENTION_DAYS,
        };

        store.init_schema().await?;

        Ok(store)
    }

    async fn init_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS architect_plans (
                id TEXT PRIMARY KEY,
                original_prompt TEXT NOT NULL,
                plan_json TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('planning', 'executing', 'completed', 'interrupted', 'failed')),
                current_subtask INTEGER NOT NULL DEFAULT 0,
                total_subtasks INTEGER NOT NULL DEFAULT 0,
                completed_results_json TEXT,
                error_message TEXT,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_plan_status ON architect_plans(status)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_plan_created ON architect_plans(created_at)")
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Save a new plan or update an existing one
    pub async fn save_plan(&self, plan: &PersistedPlan) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO architect_plans (
                id, original_prompt, plan_json, status, current_subtask,
                total_subtasks, completed_results_json, error_message, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                status = excluded.status,
                current_subtask = excluded.current_subtask,
                completed_results_json = excluded.completed_results_json,
                error_message = excluded.error_message,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(plan.id.to_string())
        .bind(&plan.original_prompt)
        .bind(&plan.plan_json)
        .bind(plan.status.to_string())
        .bind(plan.current_subtask as i64)
        .bind(plan.total_subtasks as i64)
        .bind(&plan.completed_results_json)
        .bind(&plan.error_message)
        .bind(plan.created_at.to_rfc3339())
        .bind(plan.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a plan by ID
    pub async fn get_plan(&self, id: Uuid) -> Result<Option<PersistedPlan>> {
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                i64,
                i64,
                Option<String>,
                Option<String>,
                String,
                String,
            ),
        >(
            r#"
            SELECT id, original_prompt, plan_json, status, current_subtask,
                   total_subtasks, completed_results_json, error_message, created_at, updated_at
            FROM architect_plans
            WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| PersistedPlan {
            id: Uuid::parse_str(&r.0).unwrap_or(id),
            original_prompt: r.1,
            plan_json: r.2,
            status: PlanStatus::from(r.3.as_str()),
            current_subtask: r.4 as usize,
            total_subtasks: r.5 as usize,
            completed_results_json: r.6,
            error_message: r.7,
            created_at: DateTime::parse_from_rfc3339(&r.8)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&r.9)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        }))
    }

    /// Get all interrupted plans that can be resumed
    pub async fn get_resumable_plans(&self) -> Result<Vec<PersistedPlan>> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                i64,
                i64,
                Option<String>,
                Option<String>,
                String,
                String,
            ),
        >(
            r#"
            SELECT id, original_prompt, plan_json, status, current_subtask,
                   total_subtasks, completed_results_json, error_message, created_at, updated_at
            FROM architect_plans
            WHERE status IN ('executing', 'interrupted')
            ORDER BY updated_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|r| {
                Some(PersistedPlan {
                    id: Uuid::parse_str(&r.0).ok()?,
                    original_prompt: r.1,
                    plan_json: r.2,
                    status: PlanStatus::from(r.3.as_str()),
                    current_subtask: r.4 as usize,
                    total_subtasks: r.5 as usize,
                    completed_results_json: r.6,
                    error_message: r.7,
                    created_at: DateTime::parse_from_rfc3339(&r.8)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: DateTime::parse_from_rfc3339(&r.9)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .collect())
    }

    /// Update plan status
    pub async fn update_status(&self, id: Uuid, status: PlanStatus) -> Result<()> {
        sqlx::query(
            "UPDATE architect_plans SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(status.to_string())
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Update plan progress (current subtask)
    pub async fn update_progress(
        &self,
        id: Uuid,
        current_subtask: usize,
        completed_results_json: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE architect_plans
            SET current_subtask = ?, completed_results_json = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(current_subtask as i64)
        .bind(completed_results_json)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Mark a plan as failed with an error message
    pub async fn mark_failed(&self, id: Uuid, error: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE architect_plans
            SET status = 'failed', error_message = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ?
            "#,
        )
        .bind(error)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Mark a plan as completed
    pub async fn mark_completed(&self, id: Uuid) -> Result<()> {
        self.update_status(id, PlanStatus::Completed).await
    }

    /// Mark all executing plans as interrupted (called on app startup)
    pub async fn mark_executing_as_interrupted(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE architect_plans SET status = 'interrupted', updated_at = CURRENT_TIMESTAMP WHERE status = 'executing'",
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Delete a plan by ID
    pub async fn delete_plan(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM architect_plans WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// Cleanup old completed/failed plans
    pub async fn cleanup_old_plans(&self) -> Result<u64> {
        let cutoff = Utc::now() - chrono::Duration::days(self.retention_days);

        let result = sqlx::query(
            "DELETE FROM architect_plans WHERE status IN ('completed', 'failed') AND updated_at < ?",
        )
        .bind(cutoff.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    /// Get recent plans for display in UI
    pub async fn get_recent_plans(&self, limit: usize) -> Result<Vec<PersistedPlan>> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                i64,
                i64,
                Option<String>,
                Option<String>,
                String,
                String,
            ),
        >(
            r#"
            SELECT id, original_prompt, plan_json, status, current_subtask,
                   total_subtasks, completed_results_json, error_message, created_at, updated_at
            FROM architect_plans
            ORDER BY updated_at DESC
            LIMIT ?
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|r| {
                Some(PersistedPlan {
                    id: Uuid::parse_str(&r.0).ok()?,
                    original_prompt: r.1,
                    plan_json: r.2,
                    status: PlanStatus::from(r.3.as_str()),
                    current_subtask: r.4 as usize,
                    total_subtasks: r.5 as usize,
                    completed_results_json: r.6,
                    error_message: r.7,
                    created_at: DateTime::parse_from_rfc3339(&r.8)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: DateTime::parse_from_rfc3339(&r.9)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    async fn create_test_store() -> PlanStateStore {
        static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
        let test_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let db_path = std::path::PathBuf::from(format!(
            "file:arkavo_plan_state_test_{timestamp}_{test_id}?mode=memory&cache=shared"
        ));

        PlanStateStore::new(&db_path).await.unwrap()
    }

    #[tokio::test]
    async fn test_save_and_get_plan() {
        let store = create_test_store().await;

        let plan = PersistedPlan {
            id: Uuid::new_v4(),
            original_prompt: "Build a web server".to_string(),
            plan_json: r#"{"subtasks":[]}"#.to_string(),
            status: PlanStatus::Executing,
            current_subtask: 0,
            total_subtasks: 3,
            completed_results_json: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        store.save_plan(&plan).await.unwrap();

        let retrieved = store.get_plan(plan.id).await.unwrap().unwrap();
        assert_eq!(retrieved.original_prompt, "Build a web server");
        assert_eq!(retrieved.status, PlanStatus::Executing);
        assert_eq!(retrieved.total_subtasks, 3);
    }

    #[tokio::test]
    async fn test_update_progress() {
        let store = create_test_store().await;

        let plan = PersistedPlan {
            id: Uuid::new_v4(),
            original_prompt: "Test task".to_string(),
            plan_json: "{}".to_string(),
            status: PlanStatus::Executing,
            current_subtask: 0,
            total_subtasks: 5,
            completed_results_json: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        store.save_plan(&plan).await.unwrap();
        store
            .update_progress(plan.id, 2, Some(r#"[{"result":"ok"}]"#))
            .await
            .unwrap();

        let retrieved = store.get_plan(plan.id).await.unwrap().unwrap();
        assert_eq!(retrieved.current_subtask, 2);
        assert!(retrieved.completed_results_json.is_some());
    }

    #[tokio::test]
    async fn test_get_resumable_plans() {
        let store = create_test_store().await;

        // Create an executing plan
        let executing = PersistedPlan {
            id: Uuid::new_v4(),
            original_prompt: "Executing task".to_string(),
            plan_json: "{}".to_string(),
            status: PlanStatus::Executing,
            current_subtask: 1,
            total_subtasks: 3,
            completed_results_json: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.save_plan(&executing).await.unwrap();

        // Create a completed plan (should not be resumable)
        let completed = PersistedPlan {
            id: Uuid::new_v4(),
            original_prompt: "Completed task".to_string(),
            plan_json: "{}".to_string(),
            status: PlanStatus::Completed,
            current_subtask: 3,
            total_subtasks: 3,
            completed_results_json: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.save_plan(&completed).await.unwrap();

        let resumable = store.get_resumable_plans().await.unwrap();
        assert_eq!(resumable.len(), 1);
        assert_eq!(resumable[0].original_prompt, "Executing task");
    }

    #[tokio::test]
    async fn test_mark_executing_as_interrupted() {
        let store = create_test_store().await;

        let plan = PersistedPlan {
            id: Uuid::new_v4(),
            original_prompt: "Task".to_string(),
            plan_json: "{}".to_string(),
            status: PlanStatus::Executing,
            current_subtask: 0,
            total_subtasks: 1,
            completed_results_json: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.save_plan(&plan).await.unwrap();

        let count = store.mark_executing_as_interrupted().await.unwrap();
        assert_eq!(count, 1);

        let retrieved = store.get_plan(plan.id).await.unwrap().unwrap();
        assert_eq!(retrieved.status, PlanStatus::Interrupted);
    }
}
