//! Task executor that manages task lifecycle and execution.

use crate::error::{Result, TaskError};
use crate::task_store::TaskStore;
use crate::types::{Message, Task, TaskPriority, TaskProgress, TaskStatus};
pub use arkavo_protocol::metrics::MetricsCollector;
use chrono::DateTime;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio::time::{Duration, interval};
use tracing::{debug, error, info};
use uuid::Uuid;

/// Configuration for task executor.
#[derive(Debug, Clone)]
pub struct TaskExecutorConfig {
    /// Maximum number of concurrent tasks
    pub max_concurrent_tasks: usize,
    /// Task timeout in seconds
    pub task_timeout_seconds: u64,
    /// Whether to enable metrics collection
    pub enable_metrics: bool,
}

impl Default for TaskExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 10,
            task_timeout_seconds: 300,
            enable_metrics: true,
        }
    }
}

/// Events emitted during task execution.
#[derive(Debug, Clone)]
pub enum TaskEvent {
    /// Task was submitted
    Submitted { task_id: Uuid },
    /// Task started execution
    Started { task_id: Uuid },
    /// Task completed successfully
    Completed { task_id: Uuid },
    /// Task failed
    Failed { task_id: Uuid, error: String },
    /// Task was cancelled
    Cancelled { task_id: Uuid },
}

/// Task executor that manages task lifecycle.
pub struct TaskExecutor {
    store: Arc<dyn TaskStore>,
    config: TaskExecutorConfig,
    event_tx: mpsc::UnboundedSender<TaskEvent>,
    event_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<TaskEvent>>>>,
    shutdown_tx: broadcast::Sender<()>,
    metrics: Arc<MetricsCollector>,
    task_start_times: Arc<RwLock<HashMap<Uuid, DateTime<chrono::Utc>>>>,
}

impl TaskExecutor {
    /// Create a new task executor.
    pub fn new(store: Arc<dyn TaskStore>, config: TaskExecutorConfig) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, _) = broadcast::channel(1);
        let enable_metrics = config.enable_metrics;

        Self {
            store,
            config,
            event_tx,
            event_rx: Arc::new(RwLock::new(Some(event_rx))),
            shutdown_tx,
            metrics: Arc::new(MetricsCollector::new(enable_metrics)),
            task_start_times: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new task executor with a custom metrics collector.
    pub fn with_metrics(
        store: Arc<dyn TaskStore>,
        config: TaskExecutorConfig,
        metrics: Arc<MetricsCollector>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            store,
            config,
            event_tx,
            event_rx: Arc::new(RwLock::new(Some(event_rx))),
            shutdown_tx,
            metrics,
            task_start_times: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start the task executor.
    pub fn start(&self) -> Result<()> {
        let store = Arc::clone(&self.store);
        let config = self.config.clone();
        let event_tx = self.event_tx.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let metrics = Arc::clone(&self.metrics);
        let task_start_times = Arc::clone(&self.task_start_times);

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(1));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Process pending tasks
                        if let Err(e) = process_pending_tasks(
                            &store,
                            &config,
                            &event_tx,
                            &metrics,
                            &task_start_times,
                        ).await {
                            error!("Error processing pending tasks: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Task executor shutting down");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Shutdown the task executor.
    pub fn shutdown(&self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        Ok(())
    }

    /// Submit a new task for execution.
    pub async fn submit_task(&self, message: Message) -> Result<Uuid> {
        let task_id = Uuid::new_v4();
        let task = Task {
            id: task_id,
            title: String::new(),
            description: None,
            status: TaskStatus::Submitted,
            message,
            agent_card: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            assigned_agent: None,
            parent_task: None,
            result: None,
            error: None,
            progress: None,
            priority: TaskPriority::Normal,
        };

        self.store.create_task(task).await?;

        self.event_tx
            .send(TaskEvent::Submitted { task_id })
            .map_err(|_| TaskError::Internal("Failed to send task event".to_string()))?;

        Ok(task_id)
    }

    /// Get task status.
    pub async fn get_task_status(&self, task_id: &Uuid) -> Result<TaskStatus> {
        let task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;
        Ok(task.status)
    }

    /// Update task status.
    pub async fn update_task_status(&self, task_id: &Uuid, status: TaskStatus) -> Result<()> {
        self.store.update_task_status(task_id, status).await
    }

    /// Cancel a task.
    pub async fn cancel_task(&self, task_id: &Uuid) -> Result<()> {
        self.store
            .update_task_status(task_id, TaskStatus::Canceled)
            .await?;

        self.event_tx
            .send(TaskEvent::Cancelled { task_id: *task_id })
            .map_err(|_| TaskError::Internal("Failed to send cancel event".to_string()))?;

        Ok(())
    }

    /// Update task progress.
    ///
    /// Progress reported for a task that has already finished is dropped: the
    /// work it describes is over, and the task keeps its outcome.
    pub async fn update_task_progress(&self, task_id: &Uuid, progress: TaskProgress) -> Result<()> {
        if !self.store.update_task_progress(task_id, progress).await? {
            debug!("Dropped progress for task {task_id}: it has already finished");
        }
        Ok(())
    }

    /// Fail a task with an error.
    pub async fn fail_task(
        &self,
        task_id: &Uuid,
        error: crate::types::TaskErrorInfo,
    ) -> Result<()> {
        let mut task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        task.status = TaskStatus::Failed;
        task.error = Some(error);
        task.updated_at = chrono::Utc::now();

        self.store.update_task(task).await?;

        self.event_tx
            .send(TaskEvent::Failed {
                task_id: *task_id,
                error: "Task failed".to_string(),
            })
            .map_err(|_| TaskError::Internal("Failed to send fail event".to_string()))?;

        Ok(())
    }

    /// Complete a task with a result.
    pub async fn complete_task(&self, task_id: &Uuid, result: serde_json::Value) -> Result<()> {
        let mut task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        let old_status = task.status;
        task.status = TaskStatus::Completed;
        task.result = Some(result);
        task.updated_at = chrono::Utc::now();

        self.store.update_task(task).await?;

        self.metrics
            .record_task_status_change(&format!("{old_status:?}"), "Completed");

        self.event_tx
            .send(TaskEvent::Completed { task_id: *task_id })
            .map_err(|_| TaskError::Internal("Failed to send complete event".to_string()))?;

        // Record completion time
        if let Some(start_time) = self.task_start_times.read().await.get(task_id) {
            let duration = chrono::Utc::now() - *start_time;
            let std_duration = std::time::Duration::from_secs(duration.num_seconds() as u64);
            self.metrics.record_task_completion_time(std_duration);
        }

        Ok(())
    }

    /// Get the event receiver for task events.
    pub async fn take_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<TaskEvent>> {
        self.event_rx.write().await.take()
    }
}

/// Process pending tasks from the store.
async fn process_pending_tasks(
    store: &Arc<dyn TaskStore>,
    _config: &TaskExecutorConfig,
    event_tx: &mpsc::UnboundedSender<TaskEvent>,
    metrics: &Arc<MetricsCollector>,
    task_start_times: &Arc<RwLock<HashMap<Uuid, DateTime<chrono::Utc>>>>,
) -> Result<()> {
    let pending_tasks = store.get_tasks_by_status(TaskStatus::Submitted).await?;

    for task in pending_tasks {
        // Mark task as working
        store
            .update_task_status(&task.id, TaskStatus::Working)
            .await?;

        // Record start time
        task_start_times
            .write()
            .await
            .insert(task.id, chrono::Utc::now());

        // Emit started event
        let _ = event_tx.send(TaskEvent::Started { task_id: task.id });

        // Update metrics
        metrics.record_task_status_change("Submitted", "Working");
    }

    Ok(())
}

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::task_store::SqliteTaskStore;
    use crate::types::MessagePart;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;

    /// A store that can hold one caller at the point where it has read a task
    /// and not yet acted on what it read.
    struct PausingStore {
        inner: SqliteTaskStore,
        pause_next_read: AtomicBool,
        read: Notify,
        resume: Notify,
    }

    impl PausingStore {
        async fn new() -> Arc<Self> {
            Arc::new(Self {
                inner: SqliteTaskStore::new_in_memory()
                    .await
                    .expect("in-memory task store"),
                pause_next_read: AtomicBool::new(false),
                read: Notify::new(),
                resume: Notify::new(),
            })
        }
    }

    #[async_trait]
    impl TaskStore for PausingStore {
        async fn create_task(&self, task: Task) -> Result<()> {
            self.inner.create_task(task).await
        }

        async fn get_task(&self, task_id: &Uuid) -> Result<Option<Task>> {
            let task = self.inner.get_task(task_id).await;
            if self.pause_next_read.swap(false, Ordering::SeqCst) {
                self.read.notify_one();
                self.resume.notified().await;
            }
            task
        }

        async fn update_task_status(&self, task_id: &Uuid, status: TaskStatus) -> Result<()> {
            self.inner.update_task_status(task_id, status).await
        }

        async fn list_tasks(&self, limit: Option<usize>) -> Result<Vec<Task>> {
            self.inner.list_tasks(limit).await
        }

        async fn delete_task(&self, task_id: &Uuid) -> Result<()> {
            self.inner.delete_task(task_id).await
        }

        async fn get_tasks_by_status(&self, status: TaskStatus) -> Result<Vec<Task>> {
            self.inner.get_tasks_by_status(status).await
        }

        async fn store_task_result(&self, task_id: &Uuid, result: serde_json::Value) -> Result<()> {
            self.inner.store_task_result(task_id, result).await
        }

        async fn get_task_result(&self, task_id: &Uuid) -> Result<Option<serde_json::Value>> {
            self.inner.get_task_result(task_id).await
        }

        async fn update_task(&self, task: Task) -> Result<()> {
            self.inner.update_task(task).await
        }

        async fn update_task_progress(
            &self,
            task_id: &Uuid,
            progress: TaskProgress,
        ) -> Result<bool> {
            self.inner.update_task_progress(task_id, progress).await
        }
    }

    fn executor_over(store: Arc<dyn TaskStore>) -> Arc<TaskExecutor> {
        Arc::new(TaskExecutor::new(store, TaskExecutorConfig::default()))
    }

    fn question() -> Message {
        Message {
            parts: vec![MessagePart::Text {
                content: "Which channel should we cut?".to_string(),
            }],
            metadata: None,
        }
    }

    fn finalizing() -> TaskProgress {
        TaskProgress {
            percentage: Some(95),
            message: Some("Finalizing".to_string()),
            eta_seconds: None,
        }
    }

    /// Regression: a progress update read the task, the task was completed,
    /// and the update then wrote back the row it had read. The task went back
    /// to `Working` with no result, and its requester polled until timeout.
    #[tokio::test]
    async fn progress_in_flight_does_not_undo_a_completion() {
        let store = PausingStore::new().await;
        let executor = executor_over(store.clone());
        let task_id = executor.submit_task(question()).await.expect("submitted");
        executor
            .update_task_status(&task_id, TaskStatus::Working)
            .await
            .expect("started");

        store.pause_next_read.store(true, Ordering::SeqCst);
        let mut in_flight = tokio::spawn({
            let executor = executor.clone();
            async move { executor.update_task_progress(&task_id, finalizing()).await }
        });
        // Wait for the update to be holding what it read, or to have finished
        // without reading at all.
        let finished_early = tokio::select! {
            () = store.read.notified() => false,
            outcome = &mut in_flight => {
                outcome.expect("progress update ran").expect("progress accepted");
                true
            }
        };
        store.pause_next_read.store(false, Ordering::SeqCst);

        let result = serde_json::json!({"answer": "paid social"});
        executor
            .complete_task(&task_id, result.clone())
            .await
            .expect("completed");
        store.resume.notify_one();
        if !finished_early {
            in_flight
                .await
                .expect("progress update ran")
                .expect("progress accepted");
        }

        let task = store
            .get_task(&task_id)
            .await
            .expect("readable")
            .expect("exists");
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.result, Some(result));
    }

    #[tokio::test]
    async fn progress_never_reopens_a_finished_task() {
        for finished in [
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Canceled,
            TaskStatus::Rejected,
        ] {
            let store = PausingStore::new().await;
            let executor = executor_over(store.clone());
            let task_id = executor.submit_task(question()).await.expect("submitted");
            executor
                .update_task_status(&task_id, finished)
                .await
                .expect("finished");

            executor
                .update_task_progress(&task_id, finalizing())
                .await
                .expect("late progress is not an error");

            let task = store
                .get_task(&task_id)
                .await
                .expect("readable")
                .expect("exists");
            assert_eq!(task.status, finished);
            assert!(task.progress.is_none(), "{finished:?} took late progress");
        }
    }

    #[tokio::test]
    async fn progress_on_an_open_task_is_recorded() {
        let store = PausingStore::new().await;
        let executor = executor_over(store.clone());
        let task_id = executor.submit_task(question()).await.expect("submitted");
        executor
            .update_task_status(&task_id, TaskStatus::Working)
            .await
            .expect("started");

        executor
            .update_task_progress(&task_id, finalizing())
            .await
            .expect("progress accepted");

        let task = store
            .get_task(&task_id)
            .await
            .expect("readable")
            .expect("exists");
        assert_eq!(task.status, TaskStatus::Working);
        let progress = task.progress.expect("progress recorded");
        assert_eq!(progress.percentage, Some(95));
        assert_eq!(progress.message.as_deref(), Some("Finalizing"));
    }

    #[tokio::test]
    async fn progress_for_an_unknown_task_is_an_error() {
        let executor = executor_over(PausingStore::new().await);

        let outcome = executor
            .update_task_progress(&Uuid::new_v4(), finalizing())
            .await;

        assert!(matches!(outcome, Err(TaskError::NotFound(_))));
    }
}
