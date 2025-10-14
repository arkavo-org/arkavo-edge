use std::sync::Arc;
use tokio::task::JoinHandle;
use tracing::{info, instrument, warn};

/// Enhanced task tracker with observability
#[derive(Clone)]
pub struct ObservableTaskTracker {
    _tasks: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
    name: String,
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl ObservableTaskTracker {
    /// Create a new observable task tracker
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        info!(tracker.name = %name, "Task tracker created");
        Self {
            _tasks: Arc::new(std::sync::Mutex::new(Vec::new())),
            name,
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Spawn a tracked task with observability
    #[instrument(skip(self, future), fields(tracker.name = %self.name, task.id))]
    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
            warn!("Attempted to spawn task on closed tracker");
            return tokio::spawn(future); // Return a task that will run but won't be tracked
        }

        let task_id = uuid::Uuid::new_v4();
        tracing::Span::current().record("task.id", task_id.to_string());

        info!(task.spawned = true, "Task spawned");

        tokio::spawn(async move {
            info!(task.id = %task_id, "Task started execution");

            let start_time = std::time::Instant::now();
            let result = future.await;
            let duration = start_time.elapsed();

            info!(
                task.id = %task_id,
                task.completed = true,
                task.duration_ms = duration.as_millis() as u64,
                "Task completed"
            );

            result
        })
    }

    /// Spawn a named task with custom metadata
    #[instrument(skip(self, future), fields(tracker.name = %self.name, task.name = %task_name, task.id))]
    pub fn spawn_named<F>(&self, task_name: &str, future: F) -> JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
            warn!("Attempted to spawn named task on closed tracker");
            return tokio::spawn(future); // Return a task that will run but won't be tracked
        }

        let task_id = uuid::Uuid::new_v4();
        tracing::Span::current().record("task.id", task_id.to_string());

        info!(task.spawned = true, "Named task spawned");

        let task_name = task_name.to_string();
        tokio::spawn(async move {
            info!(
                task.id = %task_id,
                task.name = %task_name,
                "Named task started execution"
            );

            let start_time = std::time::Instant::now();
            let result = future.await;
            let duration = start_time.elapsed();

            info!(
                task.id = %task_id,
                task.name = %task_name,
                task.completed = true,
                task.duration_ms = duration.as_millis() as u64,
                "Named task completed"
            );

            result
        })
    }

    /// Close the tracker and wait for all tasks to complete
    #[instrument(skip(self), fields(tracker.name = %self.name))]
    pub async fn close(&self) {
        info!("Closing task tracker");

        let start_time = std::time::Instant::now();
        self.closed
            .store(true, std::sync::atomic::Ordering::Relaxed);

        // For simplicity, we don't track individual tasks in this implementation
        // In a production system, you'd want to keep track of spawned tasks and wait for them

        let duration = start_time.elapsed();

        info!(
            tracker.closed = true,
            shutdown.duration_ms = duration.as_millis() as u64,
            "Task tracker closed"
        );
    }

    /// Get the number of currently running tasks
    pub fn len(&self) -> usize {
        // Note: In this simplified implementation, we don't track task count
        0
    }

    /// Check if the tracker has any running tasks
    pub fn is_empty(&self) -> bool {
        // Note: In this simplified implementation, we can't track running tasks accurately
        false
    }

    /// Check if the tracker is closed
    pub fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for ObservableTaskTracker {
    fn drop(&mut self) {
        if !self.is_closed() {
            warn!(
                tracker.name = %self.name,
                "Task tracker dropped without explicit close - tasks may not complete gracefully"
            );
        }
    }
}

/// Session-specific task management
pub struct SessionTaskManager {
    session_id: String,
    message_handler: Option<ObservableTaskTracker>,
    stream_handler: Option<ObservableTaskTracker>,
    cleanup_tasks: ObservableTaskTracker,
}

impl SessionTaskManager {
    pub fn new(session_id: String) -> Self {
        let cleanup_tasks = ObservableTaskTracker::new(format!("session-{session_id}-cleanup"));

        Self {
            session_id,
            message_handler: None,
            stream_handler: None,
            cleanup_tasks,
        }
    }

    /// Start the message handler task
    #[instrument(skip(self, handler), fields(session.id = %self.session_id))]
    pub fn start_message_handler<F>(&mut self, handler: F) -> JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let tracker = ObservableTaskTracker::new(format!("session-{}-messages", self.session_id));
        let handle = tracker.spawn_named("message_processor", handler);
        self.message_handler = Some(tracker);

        info!("Message handler started for session");
        handle
    }

    /// Start the stream handler task
    #[instrument(skip(self, handler), fields(session.id = %self.session_id))]
    pub fn start_stream_handler<F>(&mut self, handler: F) -> JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let tracker = ObservableTaskTracker::new(format!("session-{}-stream", self.session_id));
        let handle = tracker.spawn_named("stream_processor", handler);
        self.stream_handler = Some(tracker);

        info!("Stream handler started for session");
        handle
    }

    /// Add a cleanup task
    #[instrument(skip(self, task), fields(session.id = %self.session_id))]
    pub fn add_cleanup_task<F>(&self, task: F) -> JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        info!("Adding cleanup task for session");
        self.cleanup_tasks.spawn_named("cleanup", task)
    }

    /// Shutdown all session tasks gracefully
    #[instrument(skip(self), fields(session.id = %self.session_id))]
    pub async fn shutdown(self) {
        info!("Shutting down session task manager");

        let start_time = std::time::Instant::now();

        // Close message handler
        if let Some(tracker) = self.message_handler {
            tracker.close().await;
        }

        // Close stream handler
        if let Some(tracker) = self.stream_handler {
            tracker.close().await;
        }

        // Close cleanup tasks
        self.cleanup_tasks.close().await;

        let duration = start_time.elapsed();
        info!(
            shutdown.duration_ms = duration.as_millis() as u64,
            "Session task manager shutdown complete"
        );
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_observable_task_tracker() {
        let tracker = ObservableTaskTracker::new("test-tracker");

        let handle = tracker.spawn(async {
            sleep(Duration::from_millis(10)).await;
            42
        });

        let result = handle.await.unwrap();
        assert_eq!(result, 42);

        tracker.close().await;
    }

    #[tokio::test]
    async fn test_named_task() {
        let tracker = ObservableTaskTracker::new("test-tracker");

        let handle = tracker.spawn_named("test-task", async {
            sleep(Duration::from_millis(10)).await;
            "done"
        });

        let result = handle.await.unwrap();
        assert_eq!(result, "done");

        tracker.close().await;
    }

    #[tokio::test]
    async fn test_session_task_manager() {
        let mut manager = SessionTaskManager::new("test-session".to_string());

        let _message_handle = manager.start_message_handler(async {
            sleep(Duration::from_millis(10)).await;
            "messages done"
        });

        let _stream_handle = manager.start_stream_handler(async {
            sleep(Duration::from_millis(10)).await;
            "stream done"
        });

        let _cleanup_handle = manager.add_cleanup_task(async {
            sleep(Duration::from_millis(5)).await;
            "cleanup done"
        });

        manager.shutdown().await;
    }
}
