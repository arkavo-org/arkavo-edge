//! Golden Dataset - Benchmark tasks for HRM orchestration validation
//!
//! Provides structured test cases to validate:
//! - Conductor decomposition logic
//! - Router agent selection (Thompson Sampling)
//! - Critic verification pipeline
//! - End-to-end task completion

pub mod metrics;
pub mod runner;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Specification for a golden benchmark task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Unique task identifier (e.g., "CODE-01", "MESH-01")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Task category
    pub category: TaskCategory,
    /// Complexity level for routing decisions
    pub complexity: Complexity,
    /// The input prompt/objective for the task
    pub input: TaskInput,
    /// Expected outputs and validation rules
    pub expected: ExpectedOutput,
    /// Mock agent profiles to use
    pub agents: Vec<AgentProfile>,
    /// Optional metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Task categories for benchmarking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    /// Code transformation tasks
    Code,
    /// Multi-agent mesh tasks
    Mesh,
    /// Hybrid tasks requiring both
    Hybrid,
}

/// Complexity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    Simple,
    Medium,
    Complex,
}

/// Input specification for a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInput {
    /// The objective/prompt
    pub objective: String,
    /// Optional context files (paths relative to fixtures)
    #[serde(default)]
    pub context_files: Vec<PathBuf>,
    /// Required capabilities
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    /// Budget constraints
    #[serde(default)]
    pub budget: Option<BudgetSpec>,
}

/// Budget specification for task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetSpec {
    pub max_cost_usd: f64,
    pub max_tokens: u64,
    pub max_steps: u32,
    pub max_wall_time_ms: u64,
}

impl Default for BudgetSpec {
    fn default() -> Self {
        Self {
            max_cost_usd: 1.0,
            max_tokens: 50_000,
            max_steps: 10,
            max_wall_time_ms: 60_000,
        }
    }
}

/// Expected outputs and validation rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedOutput {
    /// Expected subtask count range
    #[serde(default)]
    pub subtask_count: Option<(u32, u32)>,
    /// Expected agent assignments
    #[serde(default)]
    pub expected_agents: Vec<String>,
    /// Validation rules to apply
    pub validation_rules: Vec<ValidationRule>,
    /// Expected final status
    pub expected_status: ExpectedStatus,
}

/// Expected final status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedStatus {
    Completed,
    Failed,
    Partial,
}

/// Validation rules for checking task output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValidationRule {
    /// Output must contain specified text
    ContainsText { text: String, case_sensitive: bool },
    /// Output must match JSON schema
    JsonSchema { schema: serde_json::Value },
    /// Output must pass regex match
    RegexMatch { pattern: String },
    /// Specific field must equal value
    FieldEquals {
        path: String,
        value: serde_json::Value,
    },
    /// Critic verification must pass
    CriticPass { checks: Vec<String> },
    /// Custom validator function name
    Custom { validator: String },
}

/// Mock agent profile for testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Agent identifier
    pub id: String,
    /// Capabilities this agent provides
    pub capabilities: Vec<String>,
    /// Simulated success rate (0.0-1.0)
    #[serde(default = "default_success_rate")]
    pub success_rate: f64,
    /// Simulated latency in ms
    #[serde(default = "default_latency")]
    pub latency_ms: u64,
    /// Canned responses for specific inputs
    #[serde(default)]
    pub responses: HashMap<String, serde_json::Value>,
}

fn default_success_rate() -> f64 {
    1.0
}

fn default_latency() -> u64 {
    50
}

/// Result of running a golden test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenResult {
    pub task_id: String,
    pub passed: bool,
    pub validation_results: Vec<ValidationResult>,
    pub metrics: metrics::TaskMetrics,
    pub error: Option<String>,
}

/// Result of a single validation rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub rule_index: usize,
    pub passed: bool,
    pub message: String,
}

/// Load a task spec from JSON file
pub fn load_task_spec(path: &std::path::Path) -> Result<TaskSpec, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let spec: TaskSpec = serde_json::from_str(&content)?;
    Ok(spec)
}

/// Load all task specs from a directory
pub fn load_all_tasks(dir: &std::path::Path) -> Result<Vec<TaskSpec>, Box<dyn std::error::Error>> {
    let mut tasks = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            tasks.push(load_task_spec(&path)?);
        }
    }
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_spec_deserialize() {
        let json = r#"{
            "id": "CODE-01",
            "name": "Simple Refactor",
            "category": "code",
            "complexity": "simple",
            "input": {
                "objective": "Rename function foo to bar"
            },
            "expected": {
                "validation_rules": [
                    {"type": "contains_text", "text": "bar", "case_sensitive": true}
                ],
                "expected_status": "completed"
            },
            "agents": []
        }"#;

        let spec: TaskSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.id, "CODE-01");
        assert_eq!(spec.category, TaskCategory::Code);
        assert_eq!(spec.complexity, Complexity::Simple);
    }

    #[test]
    fn test_validation_rule_variants() {
        let rules = vec![
            r#"{"type": "contains_text", "text": "hello", "case_sensitive": false}"#,
            r#"{"type": "regex_match", "pattern": "\\d+"}"#,
            r#"{"type": "critic_pass", "checks": ["schema", "lint"]}"#,
        ];

        for rule_json in rules {
            let rule: ValidationRule = serde_json::from_str(rule_json).unwrap();
            assert!(matches!(
                rule,
                ValidationRule::ContainsText { .. }
                    | ValidationRule::RegexMatch { .. }
                    | ValidationRule::CriticPass { .. }
            ));
        }
    }
}
