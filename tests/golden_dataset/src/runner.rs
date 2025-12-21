//! Test harness for running golden dataset benchmarks

use crate::metrics::{
    AgentCandidate, AgentSelection, CriticCheckResult, MetricsCollector, MetricsPhase,
};
use crate::{
    AgentProfile, ExpectedStatus, GoldenResult, TaskSpec, ValidationResult, ValidationRule,
};
use arkavo_critic::{CriticPipeline, LintCheck, PolicyCheck, SchemaCheck, VerificationInput};
use arkavo_hrm::burst::BurstResult;
use arkavo_hrm::conductor::Conductor;
use arkavo_hrm::schemas::TaskBudget;
use arkavo_hrm::store::InMemoryTaskStore;
use arkavo_llm::ProviderResponse;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Configuration for the test runner
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Whether to enable verbose logging
    pub verbose: bool,
    /// Maximum retries for failed tasks
    pub max_retries: u32,
    /// Whether to run Critic verification
    pub enable_critic: bool,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            max_retries: 3,
            enable_critic: true,
        }
    }
}

/// Mock agent that simulates responses
pub struct MockAgent {
    profile: AgentProfile,
    call_count: Arc<RwLock<u32>>,
}

impl MockAgent {
    pub fn new(profile: AgentProfile) -> Self {
        Self {
            profile,
            call_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Simulate executing a task
    pub async fn execute(&self, description: &str) -> (bool, serde_json::Value) {
        {
            let mut count = self.call_count.write().await;
            *count += 1;
        }

        // Simulate latency
        tokio::time::sleep(tokio::time::Duration::from_millis(self.profile.latency_ms)).await;

        // Check for canned response
        if let Some(response) = self.profile.responses.get(description) {
            return (true, response.clone());
        }

        // Use success rate to determine outcome
        let success = rand::random::<f64>() < self.profile.success_rate;

        if success {
            (
                true,
                serde_json::json!({
                    "agent": self.profile.id,
                    "description": description,
                    "status": "completed"
                }),
            )
        } else {
            (
                false,
                serde_json::json!({
                    "agent": self.profile.id,
                    "error": "Simulated failure"
                }),
            )
        }
    }

    pub fn id(&self) -> &str {
        &self.profile.id
    }

    pub fn capabilities(&self) -> &[String] {
        &self.profile.capabilities
    }
}

/// The test runner orchestrates task execution
pub struct GoldenRunner {
    config: RunnerConfig,
    agents: HashMap<String, MockAgent>,
    critic: CriticPipeline,
}

impl GoldenRunner {
    /// Create a new runner
    pub fn new(config: RunnerConfig) -> Self {
        let critic = CriticPipeline::new()
            .add_check(SchemaCheck::new())
            .add_check(PolicyCheck::with_security_defaults())
            .add_check(LintCheck::new());

        Self {
            config,
            agents: HashMap::new(),
            critic,
        }
    }

    /// Register mock agents from task spec
    pub fn with_agents(mut self, profiles: Vec<AgentProfile>) -> Self {
        for profile in profiles {
            let id = profile.id.clone();
            self.agents.insert(id, MockAgent::new(profile));
        }
        self
    }

    /// Run a single task and return the result
    pub async fn run_task(&self, spec: &TaskSpec) -> GoldenResult {
        let mut collector = MetricsCollector::new();

        // Create conductor with in-memory store
        let store = InMemoryTaskStore::new();
        let conductor = Conductor::new(store);

        // Phase 1: Decomposition
        collector.start_phase(MetricsPhase::Decomposition);

        let budget = spec
            .input
            .budget
            .as_ref()
            .map(|b| TaskBudget {
                max_cost_usd: b.max_cost_usd,
                max_tokens: b.max_tokens,
                ..TaskBudget::default()
            })
            .unwrap_or_default();

        let task = match conductor
            .create_task(spec.input.objective.clone(), budget)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                return GoldenResult {
                    task_id: spec.id.clone(),
                    passed: false,
                    validation_results: vec![],
                    metrics: collector.finalize(),
                    error: Some(format!("Failed to create task: {e}")),
                };
            }
        };

        // Simulate decomposition: create subtasks based on complexity
        let subtask_count = match spec.complexity {
            crate::Complexity::Simple => 1,
            crate::Complexity::Medium => 2,
            crate::Complexity::Complex => 3,
        };

        for i in 0..subtask_count {
            let description = format!("Subtask {}: {}", i + 1, spec.input.objective);
            let capabilities = spec.input.required_capabilities.clone();

            if let Err(e) = conductor
                .add_subtask(task.id, description, capabilities)
                .await
            {
                return GoldenResult {
                    task_id: spec.id.clone(),
                    passed: false,
                    validation_results: vec![],
                    metrics: collector.finalize(),
                    error: Some(format!("Failed to add subtask: {e}")),
                };
            }
            collector.record_subtask();
        }

        collector.end_current_phase();

        // Phase 2 & 3: Selection + Execution loop
        let mut final_output = serde_json::Value::Null;
        let mut execution_error: Option<String> = None;

        while let Ok(Some(subtask)) = conductor.next_runnable(task.id).await {
            // Selection phase
            collector.start_phase(MetricsPhase::Selection);

            let selected_agent = self.select_agent(&subtask.required_capabilities);
            let agent_id = selected_agent
                .as_ref()
                .map(|a| a.id().to_string())
                .unwrap_or_else(|| "default".to_string());

            // Record selection
            collector.record_agent_selection(AgentSelection {
                subtask_id: subtask.id.to_string(),
                selected_agent: agent_id.clone(),
                score: 1.0, // Mock score
                candidates: self
                    .agents
                    .values()
                    .map(|a| AgentCandidate {
                        agent_id: a.id().to_string(),
                        score: 0.8,
                        utility_prior: 0.5,
                    })
                    .collect(),
            });

            collector.end_current_phase();

            // Execution phase
            collector.start_phase(MetricsPhase::Execution);

            let (success, output) = if let Some(agent) = selected_agent {
                agent.execute(&subtask.description).await
            } else {
                // No agent available - simulate basic success
                (
                    true,
                    serde_json::json!({
                        "status": "completed",
                        "description": subtask.description
                    }),
                )
            };

            let contract_id = Uuid::new_v4();
            let result = if success {
                collector.record_subtask_success();
                collector.record_tokens(500);
                collector.record_cost(0.01);
                final_output = output.clone();
                BurstResult::success(contract_id, output).with_metrics(
                    1,
                    500,
                    0.01,
                    std::time::Duration::from_millis(50),
                )
            } else {
                collector.record_subtask_failure();
                execution_error = Some("Subtask execution failed".to_string());
                BurstResult::failure(contract_id, "Execution failed".to_string())
            };

            collector.end_current_phase();

            // Record result with conductor
            if let Err(e) = conductor.record_result(task.id, subtask.id, result).await {
                return GoldenResult {
                    task_id: spec.id.clone(),
                    passed: false,
                    validation_results: vec![],
                    metrics: collector.finalize(),
                    error: Some(format!("Failed to record result: {e}")),
                };
            }

            // Phase 4: Verification (if enabled)
            if self.config.enable_critic {
                collector.start_phase(MetricsPhase::Verification);

                let response = ProviderResponse {
                    content: serde_json::to_string_pretty(&final_output).unwrap_or_default(),
                    reasoning_content: None,
                    tool_calls: vec![],
                    finish_reason: None,
                };

                let input = VerificationInput::new(spec.input.objective.clone(), response, vec![]);

                let pipeline_result = self.critic.verify(&input).await;

                // Record critic results
                for evidence in &pipeline_result.evidence {
                    collector.record_critic_check(CriticCheckResult {
                        check_id: evidence.check_id.clone(),
                        passed: evidence.status.is_passed(),
                        latency_ms: evidence.latency_ms,
                        severity: Some(format!("{:?}", evidence.severity)),
                        message: Some(evidence.description.clone()),
                    });
                }

                collector.end_current_phase();

                if !pipeline_result.passed {
                    execution_error = Some("Critic verification failed".to_string());
                }
            }
        }

        // Validate results against expected output
        let validation_results =
            self.validate(&spec.expected, &final_output, execution_error.is_none());

        let all_validations_passed = validation_results.iter().all(|v| v.passed);
        let status_matches =
            self.check_status(spec.expected.expected_status, execution_error.is_none());

        // passed = validations pass AND status matches expected (which handles expected failures)
        let passed = all_validations_passed && status_matches;

        GoldenResult {
            task_id: spec.id.clone(),
            passed,
            validation_results,
            metrics: collector.finalize(),
            error: execution_error,
        }
    }

    /// Run all tasks and return results
    pub async fn run_all(&self, specs: &[TaskSpec]) -> Vec<GoldenResult> {
        let mut results = Vec::with_capacity(specs.len());
        for spec in specs {
            results.push(self.run_task(spec).await);
        }
        results
    }

    /// Select an agent based on required capabilities
    fn select_agent(&self, required_capabilities: &[String]) -> Option<&MockAgent> {
        if required_capabilities.is_empty() {
            return self.agents.values().next();
        }

        self.agents.values().find(|agent| {
            required_capabilities
                .iter()
                .all(|cap| agent.capabilities().contains(cap))
        })
    }

    /// Validate output against expected rules
    fn validate(
        &self,
        expected: &crate::ExpectedOutput,
        output: &serde_json::Value,
        success: bool,
    ) -> Vec<ValidationResult> {
        let mut results = Vec::new();
        let output_str = serde_json::to_string(output).unwrap_or_default();

        for (idx, rule) in expected.validation_rules.iter().enumerate() {
            let result = match rule {
                ValidationRule::ContainsText {
                    text,
                    case_sensitive,
                } => {
                    let haystack = if *case_sensitive {
                        output_str.clone()
                    } else {
                        output_str.to_lowercase()
                    };
                    let needle = if *case_sensitive {
                        text.clone()
                    } else {
                        text.to_lowercase()
                    };

                    ValidationResult {
                        rule_index: idx,
                        passed: haystack.contains(&needle),
                        message: format!("Contains text '{text}'"),
                    }
                }
                ValidationRule::RegexMatch { pattern } => {
                    let re = regex::Regex::new(pattern);
                    let passed = re.map(|r| r.is_match(&output_str)).unwrap_or(false);
                    ValidationResult {
                        rule_index: idx,
                        passed,
                        message: format!("Regex match '{pattern}'"),
                    }
                }
                ValidationRule::FieldEquals { path, value } => {
                    let actual = self.get_json_path(output, path);
                    let passed = actual == Some(value);
                    ValidationResult {
                        rule_index: idx,
                        passed,
                        message: format!("Field '{path}' equals expected value"),
                    }
                }
                ValidationRule::CriticPass { checks } => ValidationResult {
                    rule_index: idx,
                    passed: success, // Critic already ran
                    message: format!("Critic checks: {checks:?}"),
                },
                ValidationRule::JsonSchema { schema: _ } => {
                    // TODO: implement JSON schema validation
                    ValidationResult {
                        rule_index: idx,
                        passed: true,
                        message: "JSON schema validation (not implemented)".to_string(),
                    }
                }
                ValidationRule::Custom { validator } => ValidationResult {
                    rule_index: idx,
                    passed: true,
                    message: format!("Custom validator '{validator}' (not implemented)"),
                },
            };
            results.push(result);
        }

        results
    }

    /// Check if execution status matches expected
    fn check_status(&self, expected: ExpectedStatus, success: bool) -> bool {
        match expected {
            ExpectedStatus::Completed => success,
            ExpectedStatus::Failed => !success,
            ExpectedStatus::Partial => true, // Always accept partial
        }
    }

    /// Get a value from JSON using dot-notation path
    fn get_json_path<'a>(
        &self,
        value: &'a serde_json::Value,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        let mut current = value;
        for part in path.split('.') {
            current = current.get(part)?;
        }
        Some(current)
    }
}

impl Default for GoldenRunner {
    fn default() -> Self {
        Self::new(RunnerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BudgetSpec, Complexity, ExpectedOutput, TaskCategory, TaskInput};

    fn simple_task_spec() -> TaskSpec {
        TaskSpec {
            id: "TEST-01".to_string(),
            name: "Simple Test".to_string(),
            category: TaskCategory::Code,
            complexity: Complexity::Simple,
            input: TaskInput {
                objective: "Test objective".to_string(),
                context_files: vec![],
                required_capabilities: vec![],
                budget: Some(BudgetSpec::default()),
            },
            expected: ExpectedOutput {
                subtask_count: Some((1, 3)),
                expected_agents: vec![],
                validation_rules: vec![ValidationRule::ContainsText {
                    text: "completed".to_string(),
                    case_sensitive: false,
                }],
                expected_status: ExpectedStatus::Completed,
            },
            agents: vec![AgentProfile {
                id: "test-agent".to_string(),
                capabilities: vec![],
                success_rate: 1.0,
                latency_ms: 10,
                responses: HashMap::new(),
            }],
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_runner_simple_task() {
        let spec = simple_task_spec();
        let runner = GoldenRunner::new(RunnerConfig::default()).with_agents(spec.agents.clone());

        let result = runner.run_task(&spec).await;

        assert!(result.passed, "Task should pass: {:?}", result.error);
        assert!(result.metrics.subtask_count >= 1);
        assert!(result.metrics.total_duration_ms > 0);
    }

    #[tokio::test]
    async fn test_runner_validation() {
        let mut spec = simple_task_spec();
        spec.expected.validation_rules = vec![ValidationRule::ContainsText {
            text: "nonexistent".to_string(),
            case_sensitive: false,
        }];

        let runner = GoldenRunner::new(RunnerConfig::default()).with_agents(spec.agents.clone());
        let result = runner.run_task(&spec).await;

        assert!(!result.passed);
        assert!(!result.validation_results.is_empty());
        assert!(!result.validation_results[0].passed);
    }

    #[tokio::test]
    async fn test_runner_with_failing_agent() {
        let mut spec = simple_task_spec();
        spec.agents = vec![AgentProfile {
            id: "flaky-agent".to_string(),
            capabilities: vec![],
            success_rate: 0.0, // Always fails
            latency_ms: 10,
            responses: HashMap::new(),
        }];
        spec.expected.expected_status = ExpectedStatus::Failed;
        spec.expected.validation_rules = vec![]; // Don't validate output

        let runner = GoldenRunner::new(RunnerConfig::default()).with_agents(spec.agents.clone());
        let result = runner.run_task(&spec).await;

        // Should pass because we expected failure
        assert!(result.passed);
        assert!(result.error.is_some());
    }
}
