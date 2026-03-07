//! Main Conductor implementation for task orchestration

use crate::burst::{BurstContract, BurstResult, ContextStrategy};
use crate::conductor::LoopDetector;
use crate::error::{Error, Result};
use crate::schemas::{GlobalTaskState, SubTask, SubTaskResult, TaskBudget, TaskStatus};
use crate::store::TaskStore;
use arkavo_memory::{ContextLedger, MemoryStorage};
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_context::ContextSummarizer;

/// Default minimum context size (in characters) required to trigger offloading.
/// Contexts smaller than this threshold are not worth the overhead of archiving.
/// Based on ~500 tokens * 4 chars/token = 2000 characters.
const DEFAULT_MIN_OFFLOAD_CHARS: usize = 2000;

/// The Conductor orchestrates strategic task decomposition and execution
///
/// Key responsibilities:
/// - Decompose objectives into subtasks
/// - Manage GlobalTaskState via TaskStore
/// - Enforce loop detection and budget constraints
/// - Coordinate burst execution
pub struct Conductor<S: TaskStore> {
    store: Arc<S>,
    loop_detector: Arc<RwLock<LoopDetector>>,
    /// Context ledger for offloading large context
    context_ledger: Option<ContextLedger>,
    /// Default context strategy for new contracts
    default_context_strategy: ContextStrategy,
    /// Minimum context size (in characters) to trigger offloading
    min_offload_chars: usize,
    /// Context summarizer for generating descriptive summaries before offloading
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    context_summarizer: Option<ContextSummarizer>,
}

impl<S: TaskStore> Conductor<S> {
    /// Create a new Conductor with the given store
    pub fn new(store: S) -> Self {
        Self {
            store: Arc::new(store),
            loop_detector: Arc::new(RwLock::new(LoopDetector::new())),
            context_ledger: None,
            default_context_strategy: ContextStrategy::ArtifactReference,
            min_offload_chars: DEFAULT_MIN_OFFLOAD_CHARS,
            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            context_summarizer: None,
        }
    }

    /// Create a new Conductor with a shared store
    pub fn with_shared_store(store: Arc<S>) -> Self {
        Self {
            store,
            loop_detector: Arc::new(RwLock::new(LoopDetector::new())),
            context_ledger: None,
            default_context_strategy: ContextStrategy::ArtifactReference,
            min_offload_chars: DEFAULT_MIN_OFFLOAD_CHARS,
            #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
            context_summarizer: None,
        }
    }

    /// Enable context summarization using a local LLM model
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub async fn with_summarizer(mut self, model_path: String) -> Result<Self> {
        let summarizer = ContextSummarizer::new(model_path)
            .await
            .map_err(|e| Error::Context(e.to_string()))?;
        self.context_summarizer = Some(summarizer);
        Ok(self)
    }

    /// Set the default context strategy
    pub fn with_context_strategy(mut self, strategy: ContextStrategy) -> Self {
        self.default_context_strategy = strategy;
        self
    }

    /// Enable the context ledger using the provided memory storage
    pub fn with_ledger(mut self, storage: std::sync::Arc<MemoryStorage>) -> Self {
        self.context_ledger = Some(ContextLedger::new(storage));
        self
    }

    /// Set the minimum context size (in characters) required for offloading.
    ///
    /// Contexts smaller than this threshold will be passed through unchanged
    /// even when using `ContextStrategy::Ledger`, avoiding overhead for tiny
    /// responses like "OK" or short status messages.
    ///
    /// The default is 2000 characters (~500 tokens using the 4 chars/token heuristic).
    pub fn with_min_offload_threshold(mut self, chars: usize) -> Self {
        self.min_offload_chars = chars;
        self
    }

    /// Prepare context for a burst based on the strategy
    ///
    /// If the strategy is Ledger and a ledger is available, this will offload
    /// the context to storage and return a semantic pointer.
    ///
    /// If `summary` is `None` and a summarizer is configured, a descriptive
    /// summary will be auto-generated using a local LLM.
    pub async fn prepare_context_for_burst(
        &self,
        context: &str,
        summary: Option<&str>,
        strategy: &ContextStrategy,
    ) -> Result<String> {
        match strategy {
            ContextStrategy::Ledger => {
                // Skip offloading for small contexts - not worth the overhead
                if context.len() <= self.min_offload_chars {
                    return Ok(context.to_string());
                }

                if let Some(ledger) = &self.context_ledger {
                    let summary_str = match summary {
                        Some(s) => s.to_string(),
                        None => self.generate_summary(context).await?,
                    };
                    ledger
                        .offload(context, &summary_str, "hrm_burst_handoff")
                        .await
                        .map_err(|e| Error::Context(e.to_string()))
                } else {
                    // Fallback if ledger is not configured
                    Ok(context.to_string())
                }
            }
            _ => Ok(context.to_string()),
        }
    }

    /// Generate a descriptive summary of context using the local summarizer.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    async fn generate_summary(&self, content: &str) -> Result<String> {
        self.context_summarizer
            .as_ref()
            .ok_or_else(|| Error::Context("Summarizer not configured".to_string()))?
            .summarize(content)
            .await
            .map_err(|e| Error::Context(e.to_string()))
    }

    /// Stub for when llama-cpp feature is disabled.
    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    async fn generate_summary(&self, _content: &str) -> Result<String> {
        Err(Error::Context(
            "Context summarization requires llama-cpp feature".to_string(),
        ))
    }

    /// Create a new task
    pub async fn create_task(
        &self,
        objective: String,
        budget: TaskBudget,
    ) -> Result<GlobalTaskState> {
        let task = GlobalTaskState::new(objective, budget);

        // Persist before returning - crash recovery requirement
        self.store.save(&task).await?;

        tracing::info!(
            task_id = %task.id,
            objective = %task.objective,
            "Created new task"
        );

        Ok(task)
    }

    /// Get a task by ID
    pub async fn get_task(&self, task_id: Uuid) -> Result<GlobalTaskState> {
        self.store
            .load(task_id)
            .await?
            .ok_or(Error::TaskNotFound(task_id))
    }

    /// Transition a task from Pending to Running
    pub async fn start_task(&self, task_id: Uuid) -> Result<()> {
        let mut task = self.get_task(task_id).await?;
        if task.status == TaskStatus::Pending {
            task.status = TaskStatus::Running;
            task.updated_at = Utc::now();
            self.store.save(&task).await?;
        }
        Ok(())
    }

    /// Add a subtask to a task
    ///
    /// Checks loop detection and subtask limits before adding.
    pub async fn add_subtask(
        &self,
        task_id: Uuid,
        description: String,
        required_capabilities: Vec<String>,
    ) -> Result<SubTask> {
        let mut task = self.get_task(task_id).await?;

        // Check subtask limit
        if !task.can_add_subtask() {
            return Err(Error::MaxSubtasksExceeded {
                current: task.subtasks.len() as u32,
                max: task.max_total_subtasks,
            });
        }

        // Check for potential thrashing
        {
            let detector = self.loop_detector.read().await;
            if detector.would_thrash(&description) {
                let count = detector.failure_count(&description);
                return Err(Error::StrategicThrashing {
                    task_id,
                    description: description.clone(),
                    attempts: count + 1,
                });
            }
        }

        // Create the subtask
        let sequence = task.subtasks.len() as u32;
        let mut subtask = SubTask::new(task_id, sequence, description);
        subtask.required_capabilities = required_capabilities;

        // Add to task
        task.subtasks.push(subtask.clone());
        task.updated_at = Utc::now();

        // Persist before returning
        self.store.save(&task).await?;

        tracing::debug!(
            task_id = %task_id,
            subtask_id = %subtask.id,
            description = %subtask.description,
            "Added subtask"
        );

        Ok(subtask)
    }

    /// Create a burst contract for a subtask
    pub async fn create_contract(&self, subtask: &SubTask) -> Result<BurstContract> {
        let contract = BurstContract::with_limits(
            subtask.id,
            subtask.budget.max_steps,
            subtask.budget.max_wall_time,
            subtask.budget.max_tokens,
            subtask.budget.max_cost_usd,
        )
        .with_context_strategy(self.default_context_strategy.clone());

        tracing::debug!(
            subtask_id = %subtask.id,
            contract_id = %contract.id,
            "Created burst contract"
        );

        Ok(contract)
    }

    /// Record a burst result and update task state
    pub async fn record_result(
        &self,
        task_id: Uuid,
        subtask_id: Uuid,
        result: BurstResult,
    ) -> Result<()> {
        let mut task = self.get_task(task_id).await?;

        // Find the subtask index
        let subtask_idx = task
            .subtasks
            .iter()
            .position(|s| s.id == subtask_id)
            .ok_or(Error::SubTaskNotFound(subtask_id))?;

        // Get description for loop detector before mutating
        let description = task.subtasks[subtask_idx].description.clone();

        // Update subtask with result
        if result.is_success() {
            task.subtasks[subtask_idx].status = TaskStatus::Completed;

            // Record success in loop detector
            self.loop_detector
                .write()
                .await
                .record_success(&description);
        } else if result.is_failure() {
            task.subtasks[subtask_idx].status = TaskStatus::Failed;
            task.subtasks[subtask_idx].retry_count += 1;

            // Record failure in loop detector
            let thrashing = self
                .loop_detector
                .write()
                .await
                .record_failure(&description);

            if let Some(count) = thrashing {
                // Store hash for persistence
                let hash = LoopDetector::get_hash(&description);
                if !task.failed_subtask_hashes.contains(&hash) {
                    task.failed_subtask_hashes.push(hash);
                }

                // Don't return error here - let caller decide what to do
                tracing::warn!(
                    task_id = %task_id,
                    subtask_id = %subtask_id,
                    attempts = count,
                    "Thrashing detected"
                );
            }
        }

        // Store the result
        task.subtasks[subtask_idx].result = Some(SubTaskResult {
            success: result.is_success(),
            output: result.output.unwrap_or(serde_json::Value::Null),
            tokens_used: result.tokens_used,
            cost_usd: result.cost_usd,
            duration: result.duration,
            steps_taken: result.steps_taken,
            verification_status: crate::schemas::VerificationStatus::Pending,
            quality_score: None,
        });

        // Update task budget
        task.budget
            .record_spending(result.cost_usd, result.tokens_used, result.duration);

        // Update task status based on subtasks
        self.update_task_status(&mut task);

        if task.status.is_terminal() {
            task.intra_progress = None;
        }

        task.updated_at = Utc::now();
        task.subtasks[subtask_idx].updated_at = Utc::now();

        // Persist
        self.store.save(&task).await?;

        Ok(())
    }

    /// Update the overall task status based on subtask states
    fn update_task_status(&self, task: &mut GlobalTaskState) {
        let all_completed = task
            .subtasks
            .iter()
            .all(|s| s.status == TaskStatus::Completed);

        let any_failed = task
            .subtasks
            .iter()
            .any(|s| s.status == TaskStatus::Failed && !s.can_retry());

        let all_terminal = task.subtasks.iter().all(|s| s.status.is_terminal());

        if all_completed && !task.subtasks.is_empty() {
            task.status = TaskStatus::Completed;
        } else if any_failed && all_terminal {
            task.status = TaskStatus::Failed;
        } else if !task.subtasks.is_empty() {
            // Any subtask running OR any work remaining means we're running
            task.status = TaskStatus::Running;
        }
    }

    /// Update intra-subtask progress for smooth UI reporting
    pub async fn update_intra_progress(&self, task_id: Uuid, progress: f64) -> Result<()> {
        let mut task = self.get_task(task_id).await?;
        task.intra_progress = Some(progress.clamp(0.0, 1.0));
        task.updated_at = Utc::now();
        self.store.save(&task).await
    }

    /// Cancel a task
    pub async fn cancel_task(&self, task_id: Uuid) -> Result<()> {
        let mut task = self.get_task(task_id).await?;

        task.status = TaskStatus::Cancelled;
        for subtask in &mut task.subtasks {
            if !subtask.status.is_terminal() {
                subtask.status = TaskStatus::Cancelled;
            }
        }

        task.updated_at = Utc::now();
        self.store.save(&task).await?;

        tracing::info!(task_id = %task_id, "Cancelled task");

        Ok(())
    }

    /// Get next runnable subtask
    pub async fn next_runnable(&self, task_id: Uuid) -> Result<Option<SubTask>> {
        let task = self.get_task(task_id).await?;

        // Get IDs of completed subtasks
        let completed_ids: Vec<Uuid> = task
            .subtasks
            .iter()
            .filter(|s| s.status == TaskStatus::Completed)
            .map(|s| s.id)
            .collect();

        // Find first pending subtask with satisfied dependencies
        let runnable = task
            .subtasks
            .iter()
            .find(|s| s.status == TaskStatus::Pending && s.dependencies_met(&completed_ids))
            .cloned();

        Ok(runnable)
    }

    /// Check if a task needs more decomposition
    pub async fn needs_decomposition(&self, task_id: Uuid) -> Result<bool> {
        let task = self.get_task(task_id).await?;

        // No subtasks yet
        if task.subtasks.is_empty() {
            return Ok(true);
        }

        // All subtasks completed or failed without retries
        let all_terminal = task.subtasks.iter().all(|s| s.status.is_terminal());
        if all_terminal && task.status != TaskStatus::Completed {
            // Might need alternative decomposition
            return Ok(true);
        }

        Ok(false)
    }

    /// Get the underlying store
    pub fn store(&self) -> Arc<S> {
        Arc::clone(&self.store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InMemoryTaskStore;

    #[tokio::test]
    async fn test_create_task() {
        let store = InMemoryTaskStore::new();
        let conductor = Conductor::new(store);

        let task = conductor
            .create_task("Test objective".to_string(), TaskBudget::default())
            .await
            .unwrap();

        assert_eq!(task.objective, "Test objective");
        assert_eq!(task.status, TaskStatus::Pending);

        // Verify persisted
        let loaded = conductor.get_task(task.id).await.unwrap();
        assert_eq!(loaded.id, task.id);
    }

    #[tokio::test]
    async fn test_add_subtask() {
        let store = InMemoryTaskStore::new();
        let conductor = Conductor::new(store);

        let task = conductor
            .create_task("Test".to_string(), TaskBudget::default())
            .await
            .unwrap();

        let subtask = conductor
            .add_subtask(task.id, "Step 1".to_string(), vec![])
            .await
            .unwrap();

        assert_eq!(subtask.description, "Step 1");
        assert_eq!(subtask.sequence, 0);

        // Verify persisted
        let loaded = conductor.get_task(task.id).await.unwrap();
        assert_eq!(loaded.subtasks.len(), 1);
    }

    #[tokio::test]
    async fn test_subtask_limit() {
        let store = InMemoryTaskStore::new();
        let conductor = Conductor::new(store);

        let mut task = conductor
            .create_task("Test".to_string(), TaskBudget::default())
            .await
            .unwrap();

        task.max_total_subtasks = 2;
        conductor.store.save(&task).await.unwrap();

        // Add two subtasks
        conductor
            .add_subtask(task.id, "Step 1".to_string(), vec![])
            .await
            .unwrap();
        conductor
            .add_subtask(task.id, "Step 2".to_string(), vec![])
            .await
            .unwrap();

        // Third should fail
        let result = conductor
            .add_subtask(task.id, "Step 3".to_string(), vec![])
            .await;

        assert!(matches!(result, Err(Error::MaxSubtasksExceeded { .. })));
    }

    #[tokio::test]
    async fn test_thrashing_detection() {
        let store = InMemoryTaskStore::new();
        let conductor = Conductor::new(store);

        let task = conductor
            .create_task("Test".to_string(), TaskBudget::default())
            .await
            .unwrap();

        // First: add and fail
        let subtask1 = conductor
            .add_subtask(task.id, "Fix the bug".to_string(), vec![])
            .await
            .unwrap();
        conductor
            .record_result(
                task.id,
                subtask1.id,
                BurstResult::failure(Uuid::new_v4(), "Failed attempt 1".to_string()),
            )
            .await
            .unwrap();

        // Second: add and fail
        let subtask2 = conductor
            .add_subtask(task.id, "Fix the bug".to_string(), vec![])
            .await
            .unwrap();
        conductor
            .record_result(
                task.id,
                subtask2.id,
                BurstResult::failure(Uuid::new_v4(), "Failed attempt 2".to_string()),
            )
            .await
            .unwrap();

        // Third attempt should be blocked (would_thrash returns true after 2 failures)
        let result = conductor
            .add_subtask(task.id, "Fix the bug".to_string(), vec![])
            .await;

        assert!(matches!(result, Err(Error::StrategicThrashing { .. })));
    }

    #[tokio::test]
    async fn test_offload_threshold_skips_small_context() {
        use arkavo_memory::MemoryStorage;

        let store = InMemoryTaskStore::new();
        let memory_storage = std::sync::Arc::new(
            MemoryStorage::new_test()
                .await
                .expect("Failed to create memory storage"),
        );

        let conductor = Conductor::new(store)
            .with_ledger(memory_storage)
            .with_min_offload_threshold(100); // 100 chars minimum

        // Small context should pass through unchanged
        let small_context = "OK";
        let result = conductor
            .prepare_context_for_burst(small_context, Some("Status"), &ContextStrategy::Ledger)
            .await
            .unwrap();

        assert_eq!(
            result, small_context,
            "Small context should not be offloaded"
        );
        assert!(
            !result.contains("[ARCHIVED:"),
            "Should not contain archive marker"
        );
    }

    #[tokio::test]
    async fn test_offload_threshold_offloads_large_context() {
        use arkavo_memory::MemoryStorage;

        let store = InMemoryTaskStore::new();
        let memory_storage = std::sync::Arc::new(
            MemoryStorage::new_test()
                .await
                .expect("Failed to create memory storage"),
        );

        let conductor = Conductor::new(store)
            .with_ledger(memory_storage)
            .with_min_offload_threshold(100); // 100 chars minimum

        // Large context should be offloaded
        let large_context = "This is a large context. ".repeat(50); // ~1250 chars
        let result = conductor
            .prepare_context_for_burst(&large_context, Some("Large test"), &ContextStrategy::Ledger)
            .await
            .unwrap();

        assert!(
            result.contains("[ARCHIVED:"),
            "Large context should be offloaded"
        );
        assert_ne!(result, large_context, "Should return pointer, not original");
    }
}
