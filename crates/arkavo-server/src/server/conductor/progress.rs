//! Progress a requester sees while the conductor works on its task.
//!
//! A report is written before the conductor moves on. Reports were once
//! handed to detached tasks, which could land in any order and after the
//! conductor had returned, by which time its caller had written the outcome
//! of the task they were reporting on.

use std::sync::Arc;

use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::types::TaskProgress;
use arkavo_tasks::task_executor::TaskExecutor;
use tracing::debug;
use uuid::Uuid;

/// Where the conductor reports how far along it is.
pub(super) struct Progress<'a> {
    conductor: &'a Conductor<InMemoryTaskStore>,
    /// The task a requester polls, when the caller tracks one.
    requester_task: Option<(Uuid, &'a TaskExecutor)>,
    /// The conductor's own task, once there is one.
    hrm_task: Option<Uuid>,
}

impl<'a> Progress<'a> {
    pub(super) fn new(
        conductor: &'a Conductor<InMemoryTaskStore>,
        task_id: Option<Uuid>,
        task_executor: Option<&'a Arc<TaskExecutor>>,
    ) -> Self {
        Self {
            conductor,
            requester_task: task_id.zip(task_executor.map(Arc::as_ref)),
            hrm_task: None,
        }
    }

    /// Report on the conductor's own task as well, from here on.
    pub(super) const fn track(&mut self, hrm_task: Uuid) {
        self.hrm_task = Some(hrm_task);
    }

    /// Record that the work has reached `percentage`, doing `message`.
    ///
    /// Progress is advisory: a report that cannot be written is dropped and
    /// the work goes on.
    pub(super) async fn report(&self, message: &str, percentage: u8) {
        if let Some(hrm_task) = self.hrm_task
            && let Err(e) = self
                .conductor
                .update_intra_progress(hrm_task, f64::from(percentage) / 100.0)
                .await
        {
            debug!("Progress not recorded for HRM task {hrm_task}: {e}");
        }
        if let Some((task_id, executor)) = self.requester_task {
            let progress = TaskProgress {
                message: Some(message.to_string()),
                percentage: Some(percentage),
                eta_seconds: None,
            };
            if let Err(e) = executor.update_task_progress(&task_id, progress).await {
                debug!("Progress not recorded for task {task_id}: {e}");
            }
        }
    }
}

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which the crate's lint set
// disallows in library code. Same waiver as the other async test modules here.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_hrm::schemas::TaskBudget;
    use arkavo_protocol::types::{Message, MessagePart, TaskStatus};
    use arkavo_tasks::task_executor::TaskExecutorConfig;
    use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};

    async fn tracked_task() -> (Arc<dyn TaskStore>, Arc<TaskExecutor>, Uuid) {
        let store: Arc<dyn TaskStore> = Arc::new(
            SqliteTaskStore::new_in_memory()
                .await
                .expect("in-memory task store"),
        );
        let executor = Arc::new(TaskExecutor::new(
            store.clone(),
            TaskExecutorConfig::default(),
        ));
        let task_id = executor
            .submit_task(Message {
                parts: vec![MessagePart::Text {
                    content: "Which channel should we cut?".to_string(),
                }],
                metadata: None,
            })
            .await
            .expect("task submitted");
        executor
            .update_task_status(&task_id, TaskStatus::Working)
            .await
            .expect("task started");
        (store, executor, task_id)
    }

    /// Regression: reports were spawned and forgotten, so the conductor could
    /// return, and its caller complete the task, with a report still to be
    /// written.
    #[tokio::test]
    async fn a_report_is_written_by_the_time_it_returns() {
        let conductor = Conductor::new(InMemoryTaskStore::new());
        let (store, executor, task_id) = tracked_task().await;
        let progress = Progress::new(&conductor, Some(task_id), Some(&executor));

        progress.report("Generating LLM response", 50).await;
        progress.report("Finalizing", 95).await;

        let task = store
            .get_task(&task_id)
            .await
            .expect("task store readable")
            .expect("task exists");
        let written = task.progress.expect("progress recorded");
        assert_eq!(written.percentage, Some(95));
        assert_eq!(written.message.as_deref(), Some("Finalizing"));
    }

    #[tokio::test]
    async fn the_conductors_own_task_is_reported_on_once_tracked() {
        let conductor = Conductor::new(InMemoryTaskStore::new());
        let hrm_task = conductor
            .create_task(
                "Which channel should we cut?".to_string(),
                TaskBudget::default(),
            )
            .await
            .expect("HRM task created");
        let mut progress = Progress::new(&conductor, None, None);

        progress.report("Creating task structure", 10).await;
        let untracked = conductor.get_task(hrm_task.id).await.expect("HRM task");
        progress.track(hrm_task.id);
        progress.report("Setting up tools", 25).await;
        let tracked = conductor.get_task(hrm_task.id).await.expect("HRM task");

        assert_eq!(untracked.intra_progress, None);
        assert_eq!(tracked.intra_progress, Some(0.25));
    }
}
