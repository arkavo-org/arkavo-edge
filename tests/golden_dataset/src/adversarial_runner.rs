//! Adversarial test runner for fault tolerance validation

use crate::adversary::{AdversarialAgent, AdversaryResult, AdversaryScenario};
use crate::metrics::{CriticCheckResult, MetricsCollector, MetricsPhase};
use crate::runner::MockAgent;
use arkavo_critic::{CriticPipeline, LintCheck, PolicyCheck, SchemaCheck, VerificationInput};
use arkavo_hrm::burst::BurstResult;
use arkavo_hrm::conductor::Conductor;
use arkavo_hrm::schemas::TaskBudget;
use arkavo_hrm::store::InMemoryTaskStore;
use arkavo_llm::ProviderResponse;
use std::collections::HashMap;
use uuid::Uuid;

/// Runner for adversarial test scenarios
pub struct AdversarialRunner {
    /// Critic pipeline for verification
    critic: CriticPipeline,
    /// Maximum retries per subtask
    max_retries: u32,
}

impl AdversarialRunner {
    /// Create a new adversarial runner
    pub fn new() -> Self {
        let critic = CriticPipeline::new()
            .add_check(SchemaCheck::new())
            .add_check(PolicyCheck::with_security_defaults())
            .add_check(LintCheck::new());

        Self {
            critic,
            max_retries: 3,
        }
    }

    /// Run an adversarial scenario
    #[allow(clippy::redundant_else)]
    pub async fn run_scenario(&self, scenario: &AdversaryScenario) -> AdversaryResult {
        let mut collector = MetricsCollector::new();
        let mut total_attempts = 0u32;
        let mut faults_injected = 0u32;
        let mut retries = 0u32;
        let mut critic_rejections = 0u32;
        let mut agent_scores: HashMap<String, (u32, u32)> = HashMap::new(); // (successes, failures)

        // Create adversarial agents
        let adversaries: Vec<AdversarialAgent> = scenario
            .adversaries
            .iter()
            .map(|cfg| AdversarialAgent::new(cfg.profile.clone(), cfg.fault_rate, cfg.fault_type))
            .collect();

        // Create fallback agents
        let fallbacks: Vec<MockAgent> = scenario
            .fallback_agents
            .iter()
            .map(|p| MockAgent::new(p.clone()))
            .collect();

        // Create conductor
        let store = InMemoryTaskStore::new();
        let conductor = Conductor::new(store);

        // Create task with tight limits for thrashing detection
        let budget = TaskBudget {
            max_cost_usd: 0.10, // Low budget to trigger limits faster
            ..TaskBudget::default()
        };

        let objective = format!("Adversarial test: {}", scenario.name);
        let task = match conductor.create_task(objective.clone(), budget).await {
            Ok(t) => t,
            Err(e) => {
                return AdversaryResult {
                    scenario_id: scenario.id.clone(),
                    passed: false,
                    task_succeeded: false,
                    total_attempts: 0,
                    faults_injected: 0,
                    retries: 0,
                    critic_rejections: 0,
                    error: Some(format!("Failed to create task: {e}")),
                    agent_scores: vec![],
                };
            }
        };

        // Add a single subtask
        let subtask_desc = match scenario.id.as_str() {
            "ADV-01" => "Subtask 1: Refactor the payment module".to_string(),
            "ADV-02" => "Subtask 1: Generate a new API endpoint".to_string(),
            "ADV-03" => "Subtask 1: Fix the critical bug".to_string(),
            _ => format!("Subtask 1: {objective}"),
        };

        let capabilities = adversaries
            .first()
            .map(|a| a.capabilities().to_vec())
            .unwrap_or_default();

        // Try to add subtask - may fail with StrategicThrashing
        let subtask_result = conductor
            .add_subtask(task.id, subtask_desc.clone(), capabilities.clone())
            .await;

        let mut task_succeeded = false;
        let mut final_error: Option<String> = None;
        let mut attempt = 0;
        let mut current_subtask: Option<arkavo_hrm::schemas::SubTask> = None;

        // Main execution loop
        while attempt < self.max_retries + 1 {
            attempt += 1;
            total_attempts += 1;

            // Get or create subtask
            let subtask = if let Some(ref s) = current_subtask {
                s.clone()
            } else {
                match &subtask_result {
                    Ok(s) => {
                        current_subtask = Some(s.clone());
                        s.clone()
                    }
                    Err(e) => {
                        let err_str = format!("{e}");
                        if err_str.contains("Thrashing") {
                            final_error = Some(err_str);
                            break;
                        }
                        final_error = Some(err_str);
                        break;
                    }
                }
            };

            collector.start_phase(MetricsPhase::Execution);

            // Try adversarial agents first
            let mut found_working = false;
            let mut current_output = serde_json::Value::Null;

            for adversary in &adversaries {
                let (success, output) = adversary.execute(&subtask.description).await;

                // Track agent performance
                let entry = agent_scores
                    .entry(adversary.id().to_string())
                    .or_insert((0, 0));

                if adversary.fault_count() > faults_injected {
                    faults_injected = adversary.fault_count();
                    entry.1 += 1; // failure
                }

                if !success {
                    continue;
                }

                current_output = output;

                // Run Critic verification
                collector.start_phase(MetricsPhase::Verification);

                let response = ProviderResponse {
                    content: serde_json::to_string_pretty(&current_output).unwrap_or_default(),
                    reasoning_content: None,
                    tool_calls: vec![],
                    finish_reason: None,
                };

                let input = VerificationInput::new(objective.clone(), response, vec![]);
                let critic_result = self.critic.verify(&input).await;

                // Record critic checks
                for evidence in &critic_result.evidence {
                    collector.record_critic_check(CriticCheckResult {
                        check_id: evidence.check_id.clone(),
                        passed: evidence.status.is_passed(),
                        latency_ms: evidence.latency_us / 1000, // Convert us to ms
                        severity: Some(format!("{:?}", evidence.severity)),
                        message: Some(evidence.description.clone()),
                    });
                }

                collector.end_current_phase();

                if critic_result.passed {
                    entry.0 += 1; // success
                    found_working = true;
                    break;
                } else {
                    // Critic vetoed
                    critic_rejections += 1;
                    entry.1 += 1;
                }
            }

            // If adversaries all failed, try fallbacks
            if !found_working {
                for fallback in &fallbacks {
                    let (success, output) = fallback.execute(&subtask.description).await;

                    let entry = agent_scores
                        .entry(fallback.id().to_string())
                        .or_insert((0, 0));

                    if !success {
                        entry.1 += 1;
                        continue;
                    }

                    current_output = output;

                    // Verify fallback output
                    let response = ProviderResponse {
                        content: serde_json::to_string_pretty(&current_output).unwrap_or_default(),
                        reasoning_content: None,
                        tool_calls: vec![],
                        finish_reason: None,
                    };

                    let input = VerificationInput::new(objective.clone(), response, vec![]);
                    let critic_result = self.critic.verify(&input).await;

                    if critic_result.passed {
                        entry.0 += 1;
                        found_working = true;
                        break;
                    } else {
                        critic_rejections += 1;
                        entry.1 += 1;
                    }
                }
            }

            collector.end_current_phase();

            if found_working {
                // Record success
                let contract_id = Uuid::new_v4();
                let result = BurstResult::success(contract_id, current_output);

                if let Err(e) = conductor.record_result(task.id, subtask.id, result).await {
                    final_error = Some(format!("Failed to record result: {e}"));
                    break;
                }

                task_succeeded = true;
                break;
            } else {
                // Record failure and retry
                retries += 1;
                let contract_id = Uuid::new_v4();
                let result =
                    BurstResult::failure(contract_id, "All agents failed verification".to_string());

                if let Err(e) = conductor.record_result(task.id, subtask.id, result).await {
                    let err_str = format!("{e}");
                    if err_str.contains("Thrashing") {
                        final_error = Some(err_str);
                        break;
                    }
                }

                // Try to add new subtask for retry
                let new_subtask = conductor
                    .add_subtask(task.id, subtask_desc.clone(), capabilities.clone())
                    .await;

                match new_subtask {
                    Ok(s) => {
                        current_subtask = Some(s);
                    }
                    Err(e) => {
                        let err_str = format!("{e}");
                        if err_str.contains("Thrashing") {
                            final_error = Some(err_str);
                            break;
                        }
                    }
                }
            }
        }

        // Convert agent scores to final format
        let agent_scores_vec: Vec<(String, f64)> = agent_scores
            .into_iter()
            .map(|(id, (successes, failures))| {
                let total = successes + failures;
                let score = if total > 0 {
                    f64::from(successes) / f64::from(total)
                } else {
                    0.5
                };
                (id, score)
            })
            .collect();

        let result = AdversaryResult {
            scenario_id: scenario.id.clone(),
            passed: false, // Will be set below
            task_succeeded,
            total_attempts,
            faults_injected,
            retries,
            critic_rejections,
            error: final_error,
            agent_scores: agent_scores_vec,
        };

        // Check if result matches expectations
        let passed = result.matches_expected(&scenario.expected);

        AdversaryResult { passed, ..result }
    }
}

impl Default for AdversarialRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// Load an adversary scenario from JSON
pub fn load_scenario(
    path: &std::path::Path,
) -> Result<AdversaryScenario, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let scenario: AdversaryScenario = serde_json::from_str(&content)?;
    Ok(scenario)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adversary::{AdversaryConfig, FaultType};
    use crate::AgentProfile;
    use std::collections::HashMap;

    fn create_test_scenario(fault_type: FaultType, task_should_succeed: bool) -> AdversaryScenario {
        AdversaryScenario {
            id: "TEST".to_string(),
            name: "Test Scenario".to_string(),
            description: "Test".to_string(),
            adversaries: vec![AdversaryConfig {
                profile: AgentProfile {
                    id: "test-adversary".to_string(),
                    capabilities: vec!["test".to_string()],
                    success_rate: 1.0,
                    latency_ms: 1,
                    responses: HashMap::new(),
                },
                fault_rate: 1.0,
                fault_type,
            }],
            fallback_agents: if task_should_succeed {
                vec![AgentProfile {
                    id: "fallback".to_string(),
                    capabilities: vec!["test".to_string()],
                    success_rate: 1.0,
                    latency_ms: 1,
                    responses: HashMap::from([(
                        "Subtask 1: Adversarial test: Test Scenario".to_string(),
                        serde_json::json!({"status": "completed", "result": "success"}),
                    )]),
                }]
            } else {
                vec![]
            },
            expected: crate::adversary::ScenarioOutcome {
                task_succeeds: task_should_succeed,
                expected_error: if task_should_succeed {
                    None
                } else {
                    Some("Thrashing".to_string())
                },
                min_retries: 1,
                agent_penalized: true,
                critic_vetoed: false,
            },
        }
    }

    #[tokio::test]
    async fn test_adversarial_runner_with_recovery() {
        let scenario = create_test_scenario(FaultType::SchemaCorruption, true);
        let runner = AdversarialRunner::new();

        let result = runner.run_scenario(&scenario).await;

        assert!(result.task_succeeded, "Should recover via fallback");
        assert!(result.faults_injected > 0, "Should have injected faults");
    }

    #[tokio::test]
    async fn test_adversarial_runner_policy_violation() {
        let scenario = create_test_scenario(FaultType::PolicyViolation, true);
        let runner = AdversarialRunner::new();

        let result = runner.run_scenario(&scenario).await;

        // Policy violation should be caught by Critic
        assert!(
            result.critic_rejections > 0,
            "Critic should reject policy violation"
        );
    }
}
