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
    use crate::types::{AgentCard, Message, MessagePart, TaskPriority, TaskProgress};
    use chrono::Utc;

    /// Helper function to create a test task with all required fields
    fn create_test_task(title: &str, status: TaskStatus) -> Task {
        Task {
            id: Uuid::new_v4(),
            title: title.to_string(),
            description: Some(format!("Description for {title}")),
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            assigned_agent: None,
            parent_task: None,
            progress: None,
            error: None,
            priority: TaskPriority::Normal,
            message: Message {
                parts: vec![MessagePart::Text {
                    content: title.to_string(),
                }],
                metadata: None,
            },
            agent_card: None,
            result: None,
        }
    }

    /// Helper function to create a test task with an agent card
    fn create_test_task_with_agent(title: &str, agent_id: &str, endpoint: &str) -> Task {
        Task {
            id: Uuid::new_v4(),
            title: title.to_string(),
            description: Some(format!("Description for {title}")),
            status: TaskStatus::Submitted,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            assigned_agent: Some(agent_id.to_string()),
            parent_task: None,
            progress: None,
            error: None,
            priority: TaskPriority::High,
            message: Message {
                parts: vec![MessagePart::Text {
                    content: title.to_string(),
                }],
                metadata: None,
            },
            agent_card: Some(AgentCard {
                id: agent_id.to_string(),
                name: "Test Agent".to_string(),
                capabilities: vec!["text-generation".to_string()],
                endpoint: endpoint.to_string(),
                entitlements: vec![],
            }),
            result: None,
        }
    }

    #[tokio::test]
    async fn test_task_store_create_and_retrieve() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = create_test_task("Create Test", TaskStatus::Submitted);

        // Act
        store.create_task(task.clone()).await?;
        let retrieved = store.get_task(&task.id).await?;

        // Assert
        assert!(
            retrieved.is_some(),
            "Task should be retrievable after creation"
        );
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, task.id);
        assert_eq!(retrieved.title, task.title);
        assert_eq!(retrieved.status, TaskStatus::Submitted);

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_update_status() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = create_test_task("Update Test", TaskStatus::Submitted);
        store.create_task(task.clone()).await?;

        // Act
        store
            .update_task_status(&task.id, TaskStatus::Working)
            .await?;
        let updated = store.get_task(&task.id).await?.unwrap();

        // Assert
        assert_eq!(updated.status, TaskStatus::Working);

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_list_tasks() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task1 = create_test_task("Task 1", TaskStatus::Submitted);
        let task2 = create_test_task("Task 2", TaskStatus::Working);
        store.create_task(task1.clone()).await?;
        store.create_task(task2.clone()).await?;

        // Act
        let tasks = store.list_tasks(Some(10)).await?;

        // Assert
        assert_eq!(tasks.len(), 2, "Should list all tasks");

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_list_tasks_with_limit() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        for i in 0..5 {
            let task = create_test_task(&format!("Task {i}"), TaskStatus::Submitted);
            store.create_task(task).await?;
        }

        // Act
        let tasks = store.list_tasks(Some(3)).await?;

        // Assert
        assert_eq!(tasks.len(), 3, "Should respect the limit");

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_get_by_status() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task1 = create_test_task("Working Task", TaskStatus::Working);
        let task2 = create_test_task("Submitted Task", TaskStatus::Submitted);
        store.create_task(task1.clone()).await?;
        store.create_task(task2.clone()).await?;

        // Act
        let working_tasks = store.get_tasks_by_status(TaskStatus::Working).await?;

        // Assert
        assert_eq!(working_tasks.len(), 1);
        assert_eq!(working_tasks[0].id, task1.id);

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_store_and_retrieve_result() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = create_test_task("Result Test", TaskStatus::Working);
        store.create_task(task.clone()).await?;
        let result = serde_json::json!({"output": "Task completed successfully"});

        // Act
        store.store_task_result(&task.id, result.clone()).await?;
        let retrieved_result = store.get_task_result(&task.id).await?;

        // Assert
        assert_eq!(retrieved_result, Some(result));

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_delete() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = create_test_task("Delete Test", TaskStatus::Submitted);
        store.create_task(task.clone()).await?;

        // Act
        store.delete_task(&task.id).await?;
        let deleted = store.get_task(&task.id).await?;

        // Assert
        assert!(deleted.is_none(), "Task should be deleted");

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_with_agent_card() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = create_test_task_with_agent("Agent Task", "agent-123", "http://localhost:8080");

        // Act
        store.create_task(task.clone()).await?;
        let retrieved = store.get_task(&task.id).await?.unwrap();

        // Assert
        assert!(retrieved.agent_card.is_some());
        let agent = retrieved.agent_card.unwrap();
        assert_eq!(agent.id, "agent-123");
        assert_eq!(agent.endpoint, "http://localhost:8080");
        assert_eq!(retrieved.assigned_agent, Some("agent-123".to_string()));

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_update_nonexistent_task() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let nonexistent_id = Uuid::new_v4();

        // Act
        let result = store
            .update_task_status(&nonexistent_id, TaskStatus::Working)
            .await;

        // Assert - should return NotFound error
        assert!(
            result.is_err(),
            "Updating non-existent task should return error"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, TaskError::NotFound(_)),
            "Error should be NotFound"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_empty_list() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;

        // Act
        let tasks = store.list_tasks(Some(10)).await?;

        // Assert
        assert!(tasks.is_empty(), "Empty store should return empty list");

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_result_for_nonexistent_task() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let nonexistent_id = Uuid::new_v4();

        // Act
        let result = store.get_task_result(&nonexistent_id).await?;

        // Assert
        assert!(result.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_priority_levels() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let mut low_priority = create_test_task("Low Priority", TaskStatus::Submitted);
        low_priority.priority = TaskPriority::Low;
        let mut critical_priority = create_test_task("Critical Priority", TaskStatus::Submitted);
        critical_priority.priority = TaskPriority::Critical;

        // Act
        store.create_task(low_priority.clone()).await?;
        store.create_task(critical_priority.clone()).await?;

        let tasks = store.list_tasks(Some(10)).await?;

        // Assert
        assert_eq!(tasks.len(), 2);
        let priorities: Vec<_> = tasks.iter().map(|t| &t.priority).collect();
        assert!(priorities.contains(&&TaskPriority::Low));
        assert!(priorities.contains(&&TaskPriority::Critical));

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_parent_child_relationship() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let parent_task = create_test_task("Parent Task", TaskStatus::Working);
        store.create_task(parent_task.clone()).await?;

        let mut child_task = create_test_task("Child Task", TaskStatus::Submitted);
        child_task.parent_task = Some(parent_task.id);

        // Act
        store.create_task(child_task.clone()).await?;
        let retrieved_child = store.get_task(&child_task.id).await?.unwrap();

        // Assert
        assert_eq!(retrieved_child.parent_task, Some(parent_task.id));

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_all_status_transitions() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = create_test_task("Status Test", TaskStatus::Submitted);
        store.create_task(task.clone()).await?;

        // Act & Assert - Test all valid status transitions
        let statuses = vec![
            TaskStatus::Working,
            TaskStatus::InputRequired,
            TaskStatus::Completed,
        ];

        for status in statuses {
            store.update_task_status(&task.id, status).await?;
            let updated = store.get_task(&task.id).await?.unwrap();
            assert_eq!(updated.status, status, "Failed to transition to {status:?}");
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_error_handling() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = create_test_task("Error Test", TaskStatus::Working);
        store.create_task(task.clone()).await?;

        // Act - Update status to Failed
        store
            .update_task_status(&task.id, TaskStatus::Failed)
            .await?;

        // Note: Status update is verified
        let updated = store.get_task(&task.id).await?.unwrap();

        // Assert
        assert_eq!(updated.status, TaskStatus::Failed);

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_delete_nonexistent() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let nonexistent_id = Uuid::new_v4();

        // Act - Should return NotFound error when deleting non-existent task
        let result = store.delete_task(&nonexistent_id).await;

        // Assert
        assert!(
            result.is_err(),
            "Deleting non-existent task should return error"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, TaskError::NotFound(_)),
            "Error should be NotFound"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_complex_result() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = create_test_task("Complex Result Test", TaskStatus::Working);
        store.create_task(task.clone()).await?;

        let complex_result = serde_json::json!({
            "output": "Task completed",
            "metadata": {
                "tokens_used": 150,
                "model": "gpt-4",
                "finish_reason": "stop"
            },
            "artifacts": [
                {"type": "text", "content": "Result text"},
                {"type": "code", "language": "python", "content": "print('hello')"}
            ]
        });

        // Act
        store
            .store_task_result(&task.id, complex_result.clone())
            .await?;
        let retrieved = store.get_task_result(&task.id).await?;

        // Assert
        assert_eq!(retrieved, Some(complex_result));

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_agent_with_multiple_capabilities() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let agent_card = AgentCard {
            id: "multi-cap-agent".to_string(),
            name: "Multi-Capability Agent".to_string(),
            capabilities: vec![
                "text-generation".to_string(),
                "code-execution".to_string(),
                "file-access".to_string(),
                "web-search".to_string(),
            ],
            endpoint: "http://localhost:8080".to_string(),
            entitlements: vec![],
        };

        let task = Task {
            id: Uuid::new_v4(),
            title: "Multi-Capability Test".to_string(),
            description: Some("Test with multi-capability agent".to_string()),
            status: TaskStatus::Submitted,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            assigned_agent: Some("multi-cap-agent".to_string()),
            parent_task: None,
            progress: None,
            error: None,
            priority: TaskPriority::High,
            message: Message {
                parts: vec![MessagePart::Text {
                    content: "Test message".to_string(),
                }],
                metadata: None,
            },
            agent_card: Some(agent_card),
            result: None,
        };

        // Act
        store.create_task(task.clone()).await?;
        let retrieved = store.get_task(&task.id).await?.unwrap();

        // Assert
        assert!(retrieved.agent_card.is_some());
        let retrieved_agent = retrieved.agent_card.unwrap();
        assert_eq!(retrieved_agent.capabilities.len(), 4);
        assert!(
            retrieved_agent
                .capabilities
                .contains(&"code-execution".to_string())
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_task_store_task_with_progress() -> anyhow::Result<()> {
        // Arrange
        let store = SqliteTaskStore::new_in_memory().await?;
        let task = Task {
            id: Uuid::new_v4(),
            title: "Progress Test".to_string(),
            description: Some("Test with progress".to_string()),
            status: TaskStatus::Working,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            assigned_agent: None,
            parent_task: None,
            progress: Some(TaskProgress {
                percentage: Some(50),
                message: Some("Processing...".to_string()),
                eta_seconds: Some(30),
            }),
            error: None,
            priority: TaskPriority::Normal,
            message: Message {
                parts: vec![MessagePart::Text {
                    content: "Test".to_string(),
                }],
                metadata: None,
            },
            agent_card: None,
            result: None,
        };

        // Act
        store.create_task(task.clone()).await?;
        let retrieved = store.get_task(&task.id).await?.unwrap();

        // Assert
        assert!(retrieved.progress.is_some());
        let progress = retrieved.progress.unwrap();
        assert_eq!(progress.percentage, Some(50));
        assert_eq!(progress.message, Some("Processing...".to_string()));
        assert_eq!(progress.eta_seconds, Some(30));

        Ok(())
    }
}
