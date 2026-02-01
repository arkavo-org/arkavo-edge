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
use tracing::{error, info};
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
    pub async fn update_task_progress(&self, task_id: &Uuid, progress: TaskProgress) -> Result<()> {
        let mut task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| TaskError::NotFound(task_id.to_string()))?;

        task.progress = Some(progress);
        task.updated_at = chrono::Utc::now();

        self.store.update_task(task).await?;
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
