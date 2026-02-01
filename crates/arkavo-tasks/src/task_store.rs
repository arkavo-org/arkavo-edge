//! Persistent storage for tasks using SQLite via sqlx.

use crate::error::{Result, TaskError};
use crate::types::{Task, TaskStatus};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use std::path::Path;
use uuid::Uuid;

#[async_trait]
/// Trait defining task storage operations.
pub trait TaskStore: Send + Sync {
    /// Create a new task or update an existing one.
    async fn create_task(&self, task: Task) -> Result<()>;

    /// Get a task by its ID.
    async fn get_task(&self, task_id: &Uuid) -> Result<Option<Task>>;

    /// Update a task's status.
    async fn update_task_status(&self, task_id: &Uuid, status: TaskStatus) -> Result<()>;

    /// List all tasks, optionally limited to a count.
    async fn list_tasks(&self, limit: Option<usize>) -> Result<Vec<Task>>;

    /// Delete a task by its ID.
    async fn delete_task(&self, task_id: &Uuid) -> Result<()>;

    /// Get all tasks with a specific status.
    async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>>;

    /// Store the result for a completed task.
    async fn store_task_result(&self, task_id: &Uuid, result: serde_json::Value) -> Result<()>;

    /// Get the result for a task.
    async fn get_task_result(&self, task_id: &Uuid) -> Result<Option<serde_json::Value>>;

    /// Update a task (full replacement).
    async fn update_task(&self, task: Task) -> Result<()>;
}

/// SQLite-based implementation of task storage.
pub struct SqliteTaskStore {
    pool: SqlitePool,
}

impl SqliteTaskStore {
    /// Create a new SQLite task store at the given database path.
    pub async fn new(db_path: &Path) -> anyhow::Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Use mode=rwc to create the database if it doesn't exist
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await?;

        // Initialize database schema
        Self::init_schema(&pool).await?;

        Ok(Self { pool })
    }

    /// Create an in-memory SQLite task store for testing.
    pub async fn new_in_memory() -> anyhow::Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;

        // Initialize database schema
        Self::init_schema(&pool).await?;

        Ok(Self { pool })
    }

    async fn init_schema(pool: &SqlitePool) -> anyhow::Result<()> {
        // Create tasks table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY NOT NULL,
                data TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('submitted', 'working', 'input_required', 'completed', 'canceled', 'failed', 'rejected', 'auth_required')),
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await?;

        // Create indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status)")
            .execute(pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at DESC)")
            .execute(pool)
            .await?;

        // Create task_results table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS task_results (
                task_id TEXT PRIMARY KEY NOT NULL,
                result TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }
}

#[async_trait]
impl TaskStore for SqliteTaskStore {
    async fn create_task(&self, task: Task) -> Result<()> {
        let task_json =
            serde_json::to_string(&task).map_err(|e| TaskError::Serialization(e.to_string()))?;
        let status_str = match task.status {
            TaskStatus::Submitted => "submitted",
            TaskStatus::Working => "working",
            TaskStatus::InputRequired => "input_required",
            TaskStatus::Completed => "completed",
            TaskStatus::Canceled => "canceled",
            TaskStatus::Failed => "failed",
            TaskStatus::Rejected => "rejected",
            TaskStatus::AuthRequired => "auth_required",
        };

        sqlx::query(
            r#"
            INSERT INTO tasks (id, data, status, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO UPDATE SET
                data = excluded.data,
                status = excluded.status,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(task.id.to_string())
        .bind(task_json)
        .bind(status_str)
        .bind(task.created_at)
        .bind(task.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?;

        Ok(())
    }

    async fn get_task(&self, task_id: &Uuid) -> Result<Option<Task>> {
        let row = sqlx::query_as::<_, (String,)>(
            r#"
            SELECT data FROM tasks WHERE id = ?1
            "#,
        )
        .bind(task_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?;

        match row {
            Some((data,)) => {
                let task: Task = serde_json::from_str(&data)
                    .map_err(|e| TaskError::Serialization(e.to_string()))?;
                Ok(Some(task))
            }
            None => Ok(None),
        }
    }

    async fn update_task_status(&self, task_id: &Uuid, status: TaskStatus) -> Result<()> {
        let status_str = match status {
            TaskStatus::Submitted => "submitted",
            TaskStatus::Working => "working",
            TaskStatus::InputRequired => "input_required",
            TaskStatus::Completed => "completed",
            TaskStatus::Canceled => "canceled",
            TaskStatus::Failed => "failed",
            TaskStatus::Rejected => "rejected",
            TaskStatus::AuthRequired => "auth_required",
        };

        let now = Utc::now();

        let rows_affected = sqlx::query(
            r#"
            UPDATE tasks 
            SET status = ?1, updated_at = ?2
            WHERE id = ?3
            "#,
        )
        .bind(status_str)
        .bind(now)
        .bind(task_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?
        .rows_affected();

        if rows_affected == 0 {
            return Err(TaskError::NotFound(task_id.to_string()));
        }

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        task.status = status;
        let task_json =
            serde_json::to_string(&task).map_err(|e| TaskError::Serialization(e.to_string()))?;

        sqlx::query(
            r#"
            UPDATE tasks SET data = ?1 WHERE id = ?2
            "#,
        )
        .bind(task_json)
        .bind(task_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?;

        Ok(())
    }

    async fn list_tasks(&self, limit: Option<usize>) -> Result<Vec<Task>> {
        let limit = i64::try_from(limit.unwrap_or(100)).unwrap_or(100);

        let rows = sqlx::query_as::<_, (String,)>(
            r#"
            SELECT data FROM tasks
            ORDER BY created_at DESC
            LIMIT ?1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?;

        let tasks = rows
            .into_iter()
            .map(|(data,)| {
                serde_json::from_str::<Task>(&data)
                    .map_err(|e| TaskError::Serialization(e.to_string()))
            })
            .collect::<Result<Vec<Task>>>()?;

        Ok(tasks)
    }

    async fn delete_task(&self, task_id: &Uuid) -> Result<()> {
        let rows_affected = sqlx::query(
            r#"
            DELETE FROM tasks WHERE id = ?1
            "#,
        )
        .bind(task_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?
        .rows_affected();

        if rows_affected == 0 {
            return Err(TaskError::NotFound(task_id.to_string()));
        }

        Ok(())
    }

    async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
        let status_str = match status {
            TaskStatus::Submitted => "submitted",
            TaskStatus::Working => "working",
            TaskStatus::InputRequired => "input_required",
            TaskStatus::Completed => "completed",
            TaskStatus::Canceled => "canceled",
            TaskStatus::Failed => "failed",
            TaskStatus::Rejected => "rejected",
            TaskStatus::AuthRequired => "auth_required",
        };

        let rows = sqlx::query_as::<_, (String,)>(
            r#"
            SELECT data FROM tasks
            WHERE status = ?1
            ORDER BY created_at DESC
            "#,
        )
        .bind(status_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?;

        let tasks = rows
            .into_iter()
            .map(|(data,)| {
                serde_json::from_str::<Task>(&data)
                    .map_err(|e| TaskError::Serialization(e.to_string()))
            })
            .collect::<Result<Vec<Task>>>()?;

        Ok(tasks)
    }

    async fn store_task_result(&self, task_id: &Uuid, result: serde_json::Value) -> Result<()> {
        let result_json =
            serde_json::to_string(&result).map_err(|e| TaskError::Serialization(e.to_string()))?;
        let now = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO task_results (task_id, result, created_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(task_id) DO UPDATE SET result = excluded.result, created_at = excluded.created_at
            "#,
        )
        .bind(task_id.to_string())
        .bind(result_json)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?;

        Ok(())
    }

    async fn get_task_result(&self, task_id: &Uuid) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query_as::<_, (String,)>(
            r#"
            SELECT result FROM task_results WHERE task_id = ?1
            "#,
        )
        .bind(task_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| TaskError::Store(e.to_string()))?;

        match row {
            Some((result,)) => {
                let result: serde_json::Value = serde_json::from_str(&result)
                    .map_err(|e| TaskError::Serialization(e.to_string()))?;
                Ok(Some(result))
            }
            None => Ok(None),
        }
    }

    async fn update_task(&self, task: Task) -> Result<()> {
        // update_task is essentially the same as create_task with upsert behavior
        self.create_task(task).await
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::types::{AgentCapabilities, AgentCard, Message, MessagePart};
    use chrono::Utc;

    #[tokio::test]
    async fn test_task_store_crud() -> anyhow::Result<()> {
        let store = SqliteTaskStore::new_in_memory().await?;

        let task = Task {
            id: Uuid::new_v4(),
            status: TaskStatus::Submitted,
            message: Message {
                parts: vec![MessagePart::Text {
                    content: "Test task".to_string(),
                }],
                metadata: None,
            },
            agent_card: Some(AgentCard {
                name: "Test Agent".to_string(),
                description: Some("A test agent".to_string()),
                url: "http://localhost:8080".to_string(),
                provider: None,
                version: "1.0.0".to_string(),
                protocol_versions: vec!["0.3".to_string()],
                default_input_modes: vec!["text/plain".to_string()],
                default_output_modes: vec!["text/plain".to_string()],
                capabilities: AgentCapabilities::default(),
                skills: vec![],
                security_schemes: vec![],
                security: vec![],
                extensions: vec![],
                signature: None,
            }),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            result: None,
            error: None,
            progress: None,
        };

        store.create_task(task.clone()).await?;

        let retrieved = store.get_task(&task.id).await?;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, task.id);

        store
            .update_task_status(&task.id, TaskStatus::Working)
            .await?;
        let updated = store.get_task(&task.id).await?.unwrap();
        assert_eq!(updated.status, TaskStatus::Working);

        let tasks = store.list_tasks(Some(10)).await?;
        assert!(!tasks.is_empty());

        let working_tasks = store.get_tasks_by_status(TaskStatus::Working).await?;
        assert_eq!(working_tasks.len(), 1);

        let result = serde_json::json!({"output": "Task completed successfully"});
        store.store_task_result(&task.id, result.clone()).await?;
        let retrieved_result = store.get_task_result(&task.id).await?;
        assert_eq!(retrieved_result, Some(result));

        store.delete_task(&task.id).await?;
        let deleted = store.get_task(&task.id).await?;
        assert!(deleted.is_none());

        Ok(())
    }
}
