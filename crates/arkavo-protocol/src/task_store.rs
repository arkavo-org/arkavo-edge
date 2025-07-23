use crate::types::{AgentCard, Message, TaskError, TaskProgress, TaskStatus};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use std::path::Path;
use uuid::Uuid;

/// Complete task information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Task {
    /// Unique task identifier
    pub id: Uuid,

    /// Current task status
    pub status: TaskStatus,

    /// The original message that created this task
    pub message: Message,

    /// Agent handling this task
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_card: Option<AgentCard>,

    /// Task creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last update timestamp
    pub updated_at: DateTime<Utc>,

    /// Task result if completed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Message>,

    /// Error details if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,

    /// Progress information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskProgress>,
}

#[async_trait]
pub trait TaskStore: Send + Sync {
    async fn create_task(&self, task: Task) -> Result<()>;
    async fn get_task(&self, task_id: &Uuid) -> Result<Option<Task>>;
    async fn update_task_status(&self, task_id: &Uuid, status: TaskStatus) -> Result<()>;
    async fn list_tasks(&self, limit: Option<usize>) -> Result<Vec<Task>>;
    async fn delete_task(&self, task_id: &Uuid) -> Result<()>;
    async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>>;
    async fn store_task_result(&self, task_id: &Uuid, result: serde_json::Value) -> Result<()>;
    async fn get_task_result(&self, task_id: &Uuid) -> Result<Option<serde_json::Value>>;
}

pub struct SqliteTaskStore {
    pool: SqlitePool,
}

impl SqliteTaskStore {
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let db_url = format!("sqlite:{}", db_path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await?;

        // Initialize database schema
        Self::init_schema(&pool).await?;

        Ok(Self { pool })
    }

    pub async fn new_in_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;

        // Initialize database schema
        Self::init_schema(&pool).await?;

        Ok(Self { pool })
    }

    async fn init_schema(pool: &SqlitePool) -> Result<()> {
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
        let task_json = serde_json::to_string(&task)?;
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
            "#,
        )
        .bind(task.id.to_string())
        .bind(task_json)
        .bind(status_str)
        .bind(task.created_at)
        .bind(task.updated_at)
        .execute(&self.pool)
        .await?;

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
        .await?;

        match row {
            Some((data,)) => {
                let task: Task = serde_json::from_str(&data)?;
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
        .await?
        .rows_affected();

        if rows_affected == 0 {
            return Err(anyhow::anyhow!("Task not found"));
        }

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;
        task.status = status;
        let task_json = serde_json::to_string(&task)?;

        sqlx::query(
            r#"
            UPDATE tasks SET data = ?1 WHERE id = ?2
            "#,
        )
        .bind(task_json)
        .bind(task_id.to_string())
        .execute(&self.pool)
        .await?;

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
        .await?;

        let tasks = rows
            .into_iter()
            .map(|(data,)| serde_json::from_str(&data))
            .collect::<Result<Vec<Task>, _>>()?;

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
        .await?
        .rows_affected();

        if rows_affected == 0 {
            return Err(anyhow::anyhow!("Task not found"));
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
        .await?;

        let tasks = rows
            .into_iter()
            .map(|(data,)| serde_json::from_str(&data))
            .collect::<Result<Vec<Task>, _>>()?;

        Ok(tasks)
    }

    async fn store_task_result(&self, task_id: &Uuid, result: serde_json::Value) -> Result<()> {
        let result_json = serde_json::to_string(&result)?;
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
        .await?;

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
        .await?;

        match row {
            Some((result,)) => {
                let result: serde_json::Value = serde_json::from_str(&result)?;
                Ok(Some(result))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentIdentity, AuthenticationRequirements, InteractionMode, MessagePart};

    #[tokio::test]
    async fn test_task_store_crud() -> Result<()> {
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
                identity: AgentIdentity {
                    id: "test-agent".to_string(),
                    name: "Test Agent".to_string(),
                    description: Some("A test agent".to_string()),
                    version: "1.0.0".to_string(),
                    organization: None,
                },
                capabilities: vec![],
                authentication: AuthenticationRequirements {
                    methods: vec![],
                    required: false,
                },
                interaction_modes: vec![InteractionMode::Synchronous],
                metadata: None,
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
