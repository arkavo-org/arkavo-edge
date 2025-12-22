//! Task persistence layer for crash recovery
//!
//! The TaskStore trait defines the persistence interface that the Conductor
//! uses to persist state before dispatching bursts. This ensures tasks
//! survive process restarts.

#[cfg(feature = "file-store")]
use crate::error::Error;
use crate::error::Result;
use crate::schemas::GlobalTaskState;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Persistence interface for task state
///
/// Implementations must be thread-safe and support concurrent access.
/// The Conductor SHALL persist state transitions before dispatching bursts.
#[async_trait]
pub trait TaskStore: Send + Sync {
    /// Save or update a task state
    ///
    /// This must be called before any burst dispatch to ensure crash recovery.
    async fn save(&self, task: &GlobalTaskState) -> Result<()>;

    /// Load a task by ID
    async fn load(&self, id: Uuid) -> Result<Option<GlobalTaskState>>;

    /// Delete a task
    async fn delete(&self, id: Uuid) -> Result<()>;

    /// List all task IDs
    async fn list_ids(&self) -> Result<Vec<Uuid>>;

    /// List tasks by status
    async fn list_by_status(&self, status: crate::schemas::TaskStatus) -> Result<Vec<Uuid>>;

    /// Check if a task exists
    async fn exists(&self, id: Uuid) -> Result<bool> {
        Ok(self.load(id).await?.is_some())
    }
}

/// In-memory task store for development and testing
///
/// This implementation does not survive process restarts but is useful
/// for development, testing, and single-session workflows.
#[derive(Debug, Default)]
pub struct InMemoryTaskStore {
    tasks: Arc<RwLock<HashMap<Uuid, GlobalTaskState>>>,
}

impl InMemoryTaskStore {
    /// Create a new in-memory store
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the number of stored tasks
    pub async fn len(&self) -> usize {
        self.tasks.read().await.len()
    }

    /// Check if store is empty
    pub async fn is_empty(&self) -> bool {
        self.tasks.read().await.is_empty()
    }

    /// Clear all tasks
    pub async fn clear(&self) {
        self.tasks.write().await.clear();
    }

    /// List all tasks (returns full task states, not just IDs)
    pub async fn list_all(&self) -> Vec<GlobalTaskState> {
        self.tasks.read().await.values().cloned().collect()
    }
}

#[async_trait]
impl TaskStore for InMemoryTaskStore {
    async fn save(&self, task: &GlobalTaskState) -> Result<()> {
        let mut tasks = self.tasks.write().await;
        tasks.insert(task.id, task.clone());
        tracing::debug!(task_id = %task.id, "Saved task to in-memory store");
        Ok(())
    }

    async fn load(&self, id: Uuid) -> Result<Option<GlobalTaskState>> {
        let tasks = self.tasks.read().await;
        Ok(tasks.get(&id).cloned())
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let mut tasks = self.tasks.write().await;
        tasks.remove(&id);
        tracing::debug!(task_id = %id, "Deleted task from in-memory store");
        Ok(())
    }

    async fn list_ids(&self) -> Result<Vec<Uuid>> {
        let tasks = self.tasks.read().await;
        Ok(tasks.keys().copied().collect())
    }

    async fn list_by_status(&self, status: crate::schemas::TaskStatus) -> Result<Vec<Uuid>> {
        let tasks = self.tasks.read().await;
        Ok(tasks
            .values()
            .filter(|t| t.status == status)
            .map(|t| t.id)
            .collect())
    }
}

/// JSON file-backed task store for simple persistence
///
/// Stores each task as a separate JSON file in a directory.
/// Suitable for development and small-scale deployments.
#[cfg(feature = "file-store")]
pub struct FileTaskStore {
    directory: std::path::PathBuf,
}

#[cfg(feature = "file-store")]
impl FileTaskStore {
    /// Create a new file-backed store
    pub fn new<P: AsRef<std::path::Path>>(directory: P) -> Result<Self> {
        let dir = directory.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| Error::store(e))?;
        Ok(Self { directory: dir })
    }

    fn task_path(&self, id: Uuid) -> std::path::PathBuf {
        self.directory.join(format!("{}.json", id))
    }
}

#[cfg(feature = "file-store")]
#[async_trait]
impl TaskStore for FileTaskStore {
    async fn save(&self, task: &GlobalTaskState) -> Result<()> {
        let path = self.task_path(task.id);
        let json = serde_json::to_string_pretty(task)?;
        tokio::fs::write(&path, json)
            .await
            .map_err(|e| Error::store(e))?;
        Ok(())
    }

    async fn load(&self, id: Uuid) -> Result<Option<GlobalTaskState>> {
        let path = self.task_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let json = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| Error::store(e))?;
        let task = serde_json::from_str(&json)?;
        Ok(Some(task))
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let path = self.task_path(id);
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| Error::store(e))?;
        }
        Ok(())
    }

    async fn list_ids(&self) -> Result<Vec<Uuid>> {
        let mut ids = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.directory)
            .await
            .map_err(|e| Error::store(e))?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| Error::store(e))? {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(id_str) = name.strip_suffix(".json") {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        ids.push(id);
                    }
                }
            }
        }
        Ok(ids)
    }

    async fn list_by_status(&self, status: crate::schemas::TaskStatus) -> Result<Vec<Uuid>> {
        let ids = self.list_ids().await?;
        let mut matching = Vec::new();
        for id in ids {
            if let Some(task) = self.load(id).await? {
                if task.status == status {
                    matching.push(id);
                }
            }
        }
        Ok(matching)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::TaskBudget;

    #[tokio::test]
    async fn test_in_memory_store() {
        let store = InMemoryTaskStore::new();
        assert!(store.is_empty().await);

        let task = GlobalTaskState::new("Test task".to_string(), TaskBudget::default());
        let task_id = task.id;

        // Save
        store.save(&task).await.unwrap();
        assert_eq!(store.len().await, 1);

        // Load
        let loaded = store.load(task_id).await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().objective, "Test task");

        // List
        let ids = store.list_ids().await.unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&task_id));

        // Delete
        store.delete(task_id).await.unwrap();
        assert!(store.is_empty().await);
        assert!(store.load(task_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_store_update() {
        let store = InMemoryTaskStore::new();
        let mut task = GlobalTaskState::new("Original".to_string(), TaskBudget::default());
        let task_id = task.id;

        store.save(&task).await.unwrap();

        // Update objective
        task.objective = "Updated".to_string();
        store.save(&task).await.unwrap();

        let loaded = store.load(task_id).await.unwrap().unwrap();
        assert_eq!(loaded.objective, "Updated");

        // Still only one task
        assert_eq!(store.len().await, 1);
    }
}
