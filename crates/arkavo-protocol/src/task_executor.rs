use crate::metrics::MetricsCollector;
use crate::task_store::{Task, TaskStore};
use crate::types::{Message, TaskError, TaskProgress, TaskStatus};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio::time::{Duration, interval};
use tracing::{error, info, warn};
use uuid::Uuid;

/// Task execution event
#[derive(Debug, Clone)]
pub enum TaskEvent {
    StatusChanged {
        task_id: Uuid,
        old_status: TaskStatus,
        new_status: TaskStatus,
    },
    ProgressUpdated {
        task_id: Uuid,
        progress: TaskProgress,
    },
    ResultAvailable {
        task_id: Uuid,
        result: Message,
    },
    ErrorOccurred {
        task_id: Uuid,
        error: TaskError,
    },
}

/// Task executor configuration
#[derive(Debug, Clone)]
pub struct TaskExecutorConfig {
    /// Maximum number of concurrent tasks
    pub max_concurrent_tasks: usize,
    /// Task timeout in seconds
    pub task_timeout_seconds: u64,
    /// Interval for checking pending tasks
    pub poll_interval_ms: u64,
}

impl Default for TaskExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 10,
            task_timeout_seconds: 300, // 5 minutes
            poll_interval_ms: 1000,    // 1 second
        }
    }
}

/// Task executor manages task lifecycle and execution
pub struct TaskExecutor {
    store: Arc<dyn TaskStore>,
    config: TaskExecutorConfig,
    event_tx: mpsc::UnboundedSender<TaskEvent>,
    event_rx: Arc<RwLock<Option<mpsc::UnboundedReceiver<TaskEvent>>>>,
    shutdown_tx: broadcast::Sender<()>,
    metrics: Arc<MetricsCollector>,
    task_start_times: Arc<RwLock<HashMap<Uuid, chrono::DateTime<chrono::Utc>>>>,
}

impl TaskExecutor {
    pub fn new(store: Arc<dyn TaskStore>, config: TaskExecutorConfig) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            store,
            config,
            event_tx,
            event_rx: Arc::new(RwLock::new(Some(event_rx))),
            shutdown_tx,
            metrics: Arc::new(MetricsCollector::new(true)), // TODO: Make configurable
            task_start_times: Arc::new(RwLock::new(HashMap::new())),
        }
    }

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

    /// Start the task executor
    pub fn start(&self) -> Result<()> {
        let store = Arc::clone(&self.store);
        let config = self.config.clone();
        let event_tx = self.event_tx.clone();
        let metrics = Arc::clone(&self.metrics);
        let task_start_times = Arc::clone(&self.task_start_times);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        // Spawn the main executor loop
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(config.poll_interval_ms));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = Self::process_pending_tasks(&store, &config, &event_tx, &metrics, &task_start_times).await {
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

        // Spawn metrics updater
        let store_clone = Arc::clone(&self.store);
        let metrics_clone = Arc::clone(&self.metrics);
        let mut shutdown_rx2 = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(10)); // Update every 10 seconds

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = Self::update_task_metrics(&store_clone, &metrics_clone).await {
                            warn!("Error updating task metrics: {}", e);
                        }
                    }
                    _ = shutdown_rx2.recv() => {
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the task executor
    pub fn stop(&self) -> Result<()> {
        let _ = self.shutdown_tx.send(());
        Ok(())
    }

    /// Submit a new task for execution
    pub async fn submit_task(&self, message: Message) -> Result<Uuid> {
        let task_id = Uuid::new_v4();
        let task = Task {
            id: task_id,
            status: TaskStatus::Submitted,
            message,
            agent_card: None, // Will be assigned by the scheduler
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            result: None,
            error: None,
            progress: None,
        };

        self.store.create_task(task).await?;

        // Record metrics
        self.metrics.record_task_submitted();

        // Store task start time
        self.task_start_times
            .write()
            .await
            .insert(task_id, chrono::Utc::now());

        self.emit_event(TaskEvent::StatusChanged {
            task_id,
            old_status: TaskStatus::Submitted,
            new_status: TaskStatus::Submitted,
        });

        Ok(task_id)
    }

    /// Update task status
    pub async fn update_task_status(&self, task_id: &Uuid, new_status: TaskStatus) -> Result<()> {
        let task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;

        let old_status = task.status;
        if old_status == new_status {
            return Ok(());
        }

        // Validate state transitions
        if !Self::is_valid_transition(old_status, new_status) {
            return Err(anyhow::anyhow!(
                "Invalid status transition from {:?} to {:?}",
                old_status,
                new_status
            ));
        }

        self.store.update_task_status(task_id, new_status).await?;

        // Record metrics
        self.metrics.record_task_status_change(
            &format!("{old_status:?}").to_lowercase(),
            &format!("{new_status:?}").to_lowercase(),
        );

        // If task completed or failed, record completion time
        if matches!(
            new_status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Canceled
        ) {
            let value = self.task_start_times.write().await.remove(task_id);
            if let Some(start_time) = value {
                let duration = chrono::Utc::now().signed_duration_since(start_time);
                self.metrics
                    .record_task_completion_time(
                        Duration::from_secs(duration.num_seconds() as u64),
                    );
            }
        }

        self.emit_event(TaskEvent::StatusChanged {
            task_id: *task_id,
            old_status,
            new_status,
        });

        Ok(())
    }

    /// Update task progress
    pub async fn update_task_progress(&self, task_id: &Uuid, progress: TaskProgress) -> Result<()> {
        // Store progress in task data
        let mut task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;

        task.progress = Some(progress.clone());
        task.updated_at = chrono::Utc::now();

        // Update the entire task
        self.store.create_task(task).await?; // This will update existing

        self.emit_event(TaskEvent::ProgressUpdated {
            task_id: *task_id,
            progress,
        });

        Ok(())
    }

    /// Complete a task with a result
    pub async fn complete_task(&self, task_id: &Uuid, result: Message) -> Result<()> {
        self.store
            .store_task_result(task_id, serde_json::to_value(&result)?)
            .await?;
        self.update_task_status(task_id, TaskStatus::Completed)
            .await?;

        self.emit_event(TaskEvent::ResultAvailable {
            task_id: *task_id,
            result,
        });

        Ok(())
    }

    /// Fail a task with an error
    pub async fn fail_task(&self, task_id: &Uuid, error: TaskError) -> Result<()> {
        // Store error in task data
        let mut task = self
            .store
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Task not found"))?;

        task.error = Some(error.clone());
        task.updated_at = chrono::Utc::now();

        // Update the entire task
        self.store.create_task(task).await?; // This will update existing
        self.update_task_status(task_id, TaskStatus::Failed).await?;

        // Record error metric
        self.metrics.record_task_error(&error.code);

        self.emit_event(TaskEvent::ErrorOccurred {
            task_id: *task_id,
            error,
        });

        Ok(())
    }

    /// Get event receiver
    pub async fn get_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<TaskEvent>> {
        self.event_rx.write().await.take()
    }

    /// Process pending tasks
    async fn process_pending_tasks(
        store: &Arc<dyn TaskStore>,
        config: &TaskExecutorConfig,
        event_tx: &mpsc::UnboundedSender<TaskEvent>,
        metrics: &Arc<MetricsCollector>,
        task_start_times: &Arc<RwLock<HashMap<Uuid, chrono::DateTime<chrono::Utc>>>>,
    ) -> Result<()> {
        // Get all submitted tasks
        let submitted_tasks = store.get_tasks_by_status(TaskStatus::Submitted).await?;

        // Get current working tasks count
        let working_tasks = store.get_tasks_by_status(TaskStatus::Working).await?;
        let available_slots = config
            .max_concurrent_tasks
            .saturating_sub(working_tasks.len());

        // Start processing tasks up to available slots
        for (i, task) in submitted_tasks.iter().enumerate() {
            if i >= available_slots {
                break;
            }

            // Transition to Working status
            if let Err(e) = store
                .update_task_status(&task.id, TaskStatus::Working)
                .await
            {
                warn!("Failed to update task {} status: {}", task.id, e);
                continue;
            }

            // Store task start time if not already stored
            task_start_times
                .write()
                .await
                .entry(task.id)
                .or_insert_with(chrono::Utc::now);

            event_tx.send(TaskEvent::StatusChanged {
                task_id: task.id,
                old_status: TaskStatus::Submitted,
                new_status: TaskStatus::Working,
            })?;

            metrics.record_task_status_change("submitted", "working");

            // In a real implementation, this is where we would:
            // 1. Find an appropriate agent based on task requirements
            // 2. Send the task to the agent
            // 3. Monitor the agent's progress

            info!("Started processing task {}", task.id);
        }

        // Check for timed-out tasks
        let working_tasks = store.get_tasks_by_status(TaskStatus::Working).await?;
        let timeout_duration = Duration::from_secs(config.task_timeout_seconds);
        let now = chrono::Utc::now();

        for task in working_tasks {
            let elapsed = now.signed_duration_since(task.updated_at);
            if elapsed.num_seconds() as u64 > config.task_timeout_seconds {
                warn!(
                    "Task {} timed out after {} seconds",
                    task.id,
                    elapsed.num_seconds()
                );

                let error = TaskError {
                    code: "TIMEOUT".to_string(),
                    message: format!(
                        "Task timed out after {} seconds",
                        timeout_duration.as_secs()
                    ),
                    details: None,
                };

                // Store error and fail the task
                let mut task_copy = task.clone();
                task_copy.error = Some(error.clone());
                store.create_task(task_copy).await?;
                store
                    .update_task_status(&task.id, TaskStatus::Failed)
                    .await?;

                event_tx.send(TaskEvent::ErrorOccurred {
                    task_id: task.id,
                    error: error.clone(),
                })?;

                metrics.record_task_error(&error.code);
                metrics.record_task_status_change("working", "failed");

                // Remove from start times
                task_start_times.write().await.remove(&task.id);
            }
        }

        Ok(())
    }

    /// Update task metrics
    async fn update_task_metrics(
        store: &Arc<dyn TaskStore>,
        metrics: &Arc<MetricsCollector>,
    ) -> Result<()> {
        // Count tasks by status
        let statuses = vec![
            TaskStatus::Submitted,
            TaskStatus::Working,
            TaskStatus::InputRequired,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Canceled,
            TaskStatus::Rejected,
            TaskStatus::AuthRequired,
        ];

        for status in statuses {
            let tasks = store.get_tasks_by_status(status).await?;
            metrics.update_task_count_by_status(&format!("{status:?}").to_lowercase(), tasks.len());
        }

        Ok(())
    }

    /// Check if a status transition is valid
    fn is_valid_transition(from: TaskStatus, to: TaskStatus) -> bool {
        use TaskStatus::{
            AuthRequired, Canceled, Completed, Failed, InputRequired, Rejected, Submitted, Working,
        };

        matches!(
            (from, to),
            (Submitted | InputRequired, Working)
                | (Submitted, Rejected | Canceled)
                | (Working, Completed | Failed | Canceled | InputRequired)
                | (InputRequired, Canceled | Failed)
                | (_, AuthRequired) // Any status can transition to AuthRequired
        )
    }

    /// Emit an event
    fn emit_event(&self, event: TaskEvent) {
        if let Err(e) = self.event_tx.send(event) {
            warn!("Failed to emit task event: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_store::SqliteTaskStore;
    use crate::types::MessagePart;

    #[tokio::test]
    async fn test_task_executor_lifecycle() -> Result<()> {
        let store = Arc::new(SqliteTaskStore::new_in_memory().await?);
        let executor = TaskExecutor::new(store.clone(), TaskExecutorConfig::default());

        // Start the executor
        executor.start()?;

        // Submit a task
        let message = Message {
            parts: vec![MessagePart::Text {
                content: "Test task".to_string(),
            }],
            metadata: None,
        };
        let task_id = executor.submit_task(message).await?;

        // Check task was created
        let task = store.get_task(&task_id).await?.unwrap();
        assert_eq!(task.status, TaskStatus::Submitted);

        // Update to working
        executor
            .update_task_status(&task_id, TaskStatus::Working)
            .await?;
        let task = store.get_task(&task_id).await?.unwrap();
        assert_eq!(task.status, TaskStatus::Working);

        // Complete the task
        let result = Message {
            parts: vec![MessagePart::Text {
                content: "Task completed".to_string(),
            }],
            metadata: None,
        };
        executor.complete_task(&task_id, result).await?;

        let task = store.get_task(&task_id).await?.unwrap();
        assert_eq!(task.status, TaskStatus::Completed);

        // Stop the executor
        executor.stop()?;

        Ok(())
    }

    #[test]
    fn test_valid_transitions() {
        use TaskStatus::*;

        assert!(TaskExecutor::is_valid_transition(Submitted, Working));
        assert!(TaskExecutor::is_valid_transition(Working, Completed));
        assert!(TaskExecutor::is_valid_transition(Working, Failed));
        assert!(TaskExecutor::is_valid_transition(InputRequired, Working));

        assert!(!TaskExecutor::is_valid_transition(Completed, Working));
        assert!(!TaskExecutor::is_valid_transition(Failed, Completed));
    }
}
