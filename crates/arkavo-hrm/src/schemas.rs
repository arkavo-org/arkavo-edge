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
    /// Unique identifier for this task
    pub id: Uuid,
    /// Human-readable description of the objective
    pub objective: String,
    /// Current overall status
    pub status: TaskStatus,
    /// Priority level
    pub priority: Priority,
    /// Decomposed subtasks
    pub subtasks: Vec<SubTask>,
    /// When the task was created
    pub created_at: DateTime<Utc>,
    /// When the task was last updated
    pub updated_at: DateTime<Utc>,
    /// Deadline if set
    pub deadline: Option<DateTime<Utc>>,
    /// Overall budget constraints
    pub budget: TaskBudget,
    /// Task-level metadata
    pub metadata: HashMap<String, serde_json::Value>,
    /// Parent task ID if this is a nested task
    pub parent_id: Option<Uuid>,
    /// Correlation ID for tracing
    pub correlation_id: String,

    // Loop detection guardrails
    /// Maximum total subtasks allowed (prevents infinite decomposition)
    pub max_total_subtasks: u32,
    /// Current recursion depth
    pub recursion_depth: u32,
    /// Hashes of failed subtask descriptions (for thrashing detection)
    pub failed_subtask_hashes: Vec<u64>,
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
    pub fn progress(&self) -> f64 {
        if self.subtasks.is_empty() {
            return 0.0;
        }
        self.completed_count() as f64 / self.subtasks.len() as f64
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
            max_wall_time: Duration::from_secs(3600), // 1 hour
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
    /// Unique identifier
    pub id: Uuid,
    /// Parent task ID
    pub parent_id: Uuid,
    /// Sequence number within parent
    pub sequence: u32,
    /// Description of what this subtask does
    pub description: String,
    /// Current status
    pub status: TaskStatus,
    /// Required capabilities to execute
    pub required_capabilities: Vec<String>,
    /// IDs of subtasks that must complete first
    pub dependencies: Vec<Uuid>,
    /// Assigned agent ID (if scheduled)
    pub assigned_agent: Option<String>,
    /// Assigned model (if scheduled)
    pub assigned_model: Option<String>,
    /// Individual budget for this subtask
    pub budget: SubTaskBudget,
    /// Results from execution
    pub result: Option<SubTaskResult>,
    /// Retry count
    pub retry_count: u32,
    /// Maximum retries allowed
    pub max_retries: u32,
    /// When created
    pub created_at: DateTime<Utc>,
    /// When last updated
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
    fn test_task_state_creation() {
        let task = GlobalTaskState::new("Test objective".to_string(), TaskBudget::default());
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.subtasks.is_empty());
        assert!(task.can_add_subtask());
    }

    #[test]
    fn test_task_budget_tracking() {
        let mut budget = TaskBudget::default();
        assert!(budget.has_remaining());

        budget.record_spending(5.0, 50_000, Duration::from_secs(1800));
        assert!(budget.has_remaining());
        assert_eq!(budget.remaining_cost(), 5.0);

        budget.record_spending(6.0, 0, Duration::ZERO);
        assert!(!budget.has_remaining());
    }

    #[test]
    fn test_subtask_dependencies() {
        let parent_id = Uuid::new_v4();
        let mut subtask = SubTask::new(parent_id, 0, "Test subtask".to_string());

        let dep1 = Uuid::new_v4();
        let dep2 = Uuid::new_v4();
        subtask.dependencies = vec![dep1, dep2];

        assert!(!subtask.dependencies_met(&[dep1]));
        assert!(subtask.dependencies_met(&[dep1, dep2]));
        assert!(subtask.dependencies_met(&[dep1, dep2, Uuid::new_v4()]));
    }
}
