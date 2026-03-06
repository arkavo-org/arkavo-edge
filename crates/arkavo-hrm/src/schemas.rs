//! Core data structures for HRM orchestration

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

/// Priority levels for task execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

/// Current status of a task or subtask
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TaskStatus {
    #[default]
    Pending,
    Scheduled,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
    /// Suspended awaiting human approval (HITL)
    Suspended,
}

impl TaskStatus {
    /// Check if the status is terminal (no more work expected)
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Check if the status allows new work
    pub fn can_progress(&self) -> bool {
        matches!(self, Self::Pending | Self::Scheduled | Self::Running)
    }
}

/// Global state for the entire orchestration objective
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalTaskState {
    pub id: Uuid,
    pub objective: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub subtasks: Vec<SubTask>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deadline: Option<DateTime<Utc>>,
    pub budget: TaskBudget,
    pub metadata: HashMap<String, serde_json::Value>,
    pub parent_id: Option<Uuid>,
    pub correlation_id: String,
    /// Maximum total subtasks allowed (prevents infinite decomposition)
    pub max_total_subtasks: u32,
    pub recursion_depth: u32,
    pub failed_subtask_hashes: Vec<u64>,
    /// Intra-subtask progress (0.0-1.0) for smooth UI reporting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intra_progress: Option<f64>,
}

impl GlobalTaskState {
    /// Create a new task state
    pub fn new(objective: String, budget: TaskBudget) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            objective,
            status: TaskStatus::Pending,
            priority: Priority::Normal,
            subtasks: Vec::new(),
            created_at: now,
            updated_at: now,
            deadline: None,
            budget,
            metadata: HashMap::new(),
            parent_id: None,
            correlation_id: Uuid::new_v4().to_string(),
            max_total_subtasks: 50,
            recursion_depth: 0,
            failed_subtask_hashes: Vec::new(),
            intra_progress: None,
        }
    }

    /// Check if adding more subtasks would exceed limits
    pub fn can_add_subtask(&self) -> bool {
        (self.subtasks.len() as u32) < self.max_total_subtasks
    }

    /// Get count of completed subtasks
    pub fn completed_count(&self) -> usize {
        self.subtasks
            .iter()
            .filter(|s| s.status == TaskStatus::Completed)
            .count()
    }

    /// Get count of failed subtasks
    pub fn failed_count(&self) -> usize {
        self.subtasks
            .iter()
            .filter(|s| s.status == TaskStatus::Failed)
            .count()
    }

    /// Calculate overall progress (0.0 to 1.0)
    ///
    /// Blends subtask completion with intra-subtask progress for smooth UI updates.
    pub fn progress(&self) -> f64 {
        if self.subtasks.is_empty() {
            return self.intra_progress.unwrap_or(0.0);
        }
        let completed = self.completed_count() as f64;
        let total = self.subtasks.len() as f64;
        let partial = if self.status == TaskStatus::Running {
            self.intra_progress.unwrap_or(0.0) / total
        } else {
            0.0
        };
        (completed / total) + partial
    }
}

/// Budget constraints for a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskBudget {
    /// Maximum total cost in USD
    pub max_cost_usd: f64,
    /// Cost spent so far
    pub spent_usd: f64,
    /// Maximum total tokens
    pub max_tokens: u64,
    /// Tokens used so far
    pub tokens_used: u64,
    /// Maximum wall time
    #[serde(with = "duration_serde")]
    pub max_wall_time: Duration,
    /// Time elapsed
    #[serde(with = "duration_serde")]
    pub elapsed: Duration,
}

impl Default for TaskBudget {
    fn default() -> Self {
        Self {
            max_cost_usd: 10.0,
            spent_usd: 0.0,
            max_tokens: 100_000,
            tokens_used: 0,
            max_wall_time: Duration::from_secs(3600),
            elapsed: Duration::ZERO,
        }
    }
}

impl TaskBudget {
    /// Check if budget allows more spending
    pub fn has_remaining(&self) -> bool {
        self.spent_usd < self.max_cost_usd
            && self.tokens_used < self.max_tokens
            && self.elapsed < self.max_wall_time
    }

    /// Remaining cost budget
    pub fn remaining_cost(&self) -> f64 {
        (self.max_cost_usd - self.spent_usd).max(0.0)
    }

    /// Remaining token budget
    pub fn remaining_tokens(&self) -> u64 {
        self.max_tokens.saturating_sub(self.tokens_used)
    }

    /// Record spending
    pub fn record_spending(&mut self, cost_usd: f64, tokens: u64, elapsed: Duration) {
        self.spent_usd += cost_usd;
        self.tokens_used += tokens;
        self.elapsed += elapsed;
    }
}

/// A decomposed unit of work
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: Uuid,
    pub parent_id: Uuid,
    pub sequence: u32,
    pub description: String,
    pub status: TaskStatus,
    pub required_capabilities: Vec<String>,
    pub dependencies: Vec<Uuid>,
    pub assigned_agent: Option<String>,
    pub assigned_model: Option<String>,
    pub budget: SubTaskBudget,
    pub result: Option<SubTaskResult>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SubTask {
    /// Create a new subtask
    pub fn new(parent_id: Uuid, sequence: u32, description: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            parent_id,
            sequence,
            description,
            status: TaskStatus::Pending,
            required_capabilities: Vec::new(),
            dependencies: Vec::new(),
            assigned_agent: None,
            assigned_model: None,
            budget: SubTaskBudget::default(),
            result: None,
            retry_count: 0,
            max_retries: 3,
            created_at: now,
            updated_at: now,
        }
    }

    /// Check if this subtask can be retried
    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries && self.status == TaskStatus::Failed
    }

    /// Check if dependencies are satisfied
    pub fn dependencies_met(&self, completed_ids: &[Uuid]) -> bool {
        self.dependencies
            .iter()
            .all(|dep| completed_ids.contains(dep))
    }
}

/// Budget for a single subtask
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTaskBudget {
    pub max_steps: u32,
    #[serde(with = "duration_serde")]
    pub max_wall_time: Duration,
    pub max_cost_usd: f64,
    pub max_tokens: u64,
}

impl Default for SubTaskBudget {
    fn default() -> Self {
        Self {
            max_steps: 3,
            max_wall_time: Duration::from_secs(45),
            max_cost_usd: 0.50,
            max_tokens: 10_000,
        }
    }
}

/// Result of subtask execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTaskResult {
    pub success: bool,
    pub output: serde_json::Value,
    pub tokens_used: u64,
    pub cost_usd: f64,
    #[serde(with = "duration_serde")]
    pub duration: Duration,
    pub steps_taken: u32,
    pub verification_status: VerificationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f64>,
}

/// Verification outcome from the Critic
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum VerificationStatus {
    #[default]
    Pending,
    Passed,
    Failed {
        reason: String,
    },
    Partial {
        score: f64,
        notes: String,
    },
}

impl VerificationStatus {
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Serde support for Duration
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_terminal_and_progress() {
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(!TaskStatus::Pending.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
        assert!(TaskStatus::Pending.can_progress());
        assert!(TaskStatus::Running.can_progress());
        assert!(!TaskStatus::Completed.can_progress());
        assert!(!TaskStatus::Suspended.can_progress());
        assert_eq!(TaskStatus::default(), TaskStatus::Pending);
    }

    #[test]
    fn test_priority() {
        assert!(Priority::Low < Priority::Normal);
        assert!(Priority::Normal < Priority::High);
        assert!(Priority::High < Priority::Critical);
        assert_eq!(Priority::default(), Priority::Normal);
    }

    #[test]
    fn test_global_task_state() {
        let mut task = GlobalTaskState::new("obj".to_string(), TaskBudget::default());
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.subtasks.is_empty());
        assert!(task.can_add_subtask());
        assert_eq!(task.progress(), 0.0);
        task.max_total_subtasks = 1;
        task.subtasks
            .push(SubTask::new(task.id, 0, "a".to_string()));
        assert!(!task.can_add_subtask());
    }

    #[test]
    fn test_task_counts_and_progress() {
        let mut task = GlobalTaskState::new("t".to_string(), TaskBudget::default());
        let mut s1 = SubTask::new(task.id, 0, "a".to_string());
        s1.status = TaskStatus::Completed;
        let mut s2 = SubTask::new(task.id, 1, "b".to_string());
        s2.status = TaskStatus::Failed;
        task.subtasks = vec![s1, s2];
        assert_eq!(task.completed_count(), 1);
        assert_eq!(task.failed_count(), 1);
        assert!((task.progress() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_task_budget() {
        let mut b = TaskBudget::default();
        assert!(b.has_remaining());
        b.record_spending(5.0, 50_000, Duration::from_secs(1800));
        assert_eq!(b.remaining_cost(), 5.0);
        assert_eq!(b.remaining_tokens(), 50_000);
        b.record_spending(6.0, 0, Duration::ZERO);
        assert!(!b.has_remaining());
        assert_eq!(b.remaining_cost(), 0.0);
    }

    #[test]
    fn test_subtask() {
        let pid = Uuid::new_v4();
        let mut s = SubTask::new(pid, 0, "t".to_string());
        assert!(!s.can_retry());
        s.status = TaskStatus::Failed;
        assert!(s.can_retry());
        s.retry_count = 3;
        assert!(!s.can_retry());
        let d1 = Uuid::new_v4();
        s.dependencies = vec![d1];
        assert!(!s.dependencies_met(&[]));
        assert!(s.dependencies_met(&[d1]));
    }

    #[test]
    fn test_verification_status() {
        assert!(VerificationStatus::Passed.is_passed());
        assert!(!VerificationStatus::Pending.is_passed());
        assert!(VerificationStatus::Failed { reason: "x".into() }.is_failed());
        assert!(!VerificationStatus::Passed.is_failed());
    }

    #[test]
    fn test_intra_progress_blending() {
        let mut task = GlobalTaskState::new("t".to_string(), TaskBudget::default());
        task.subtasks
            .push(SubTask::new(task.id, 0, "a".to_string()));
        task.status = TaskStatus::Running;

        // No intra_progress → 0.0
        assert!((task.progress() - 0.0).abs() < f64::EPSILON);

        // 40% intra_progress on 1 subtask → 0.4
        task.intra_progress = Some(0.4);
        assert!((task.progress() - 0.4).abs() < f64::EPSILON);

        // Completed task ignores intra_progress
        task.status = TaskStatus::Completed;
        task.subtasks[0].status = TaskStatus::Completed;
        task.intra_progress = None;
        assert!((task.progress() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_intra_progress_empty_subtasks() {
        let mut task = GlobalTaskState::new("t".to_string(), TaskBudget::default());
        assert!((task.progress() - 0.0).abs() < f64::EPSILON);
        task.intra_progress = Some(0.25);
        assert!((task.progress() - 0.25).abs() < f64::EPSILON);
    }
}
