//! Adversarial agents for testing fault tolerance
//!
//! These agents inject various failure modes to validate:
//! - Conductor retry logic
//! - Router penalization (Thompson Sampling)
//! - Critic verification gates
//! - Loop detection guardrails

use crate::AgentProfile;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Types of faults an adversarial agent can inject
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultType {
    /// Returns valid JSON but with policy violations (secrets, PII)
    PolicyViolation,
    /// Returns garbled/invalid JSON that fails schema validation
    SchemaCorruption,
    /// Returns syntactically valid but semantically useless response
    LazyCompliance,
    /// Simulates network timeout or agent crash
    Timeout,
    /// Returns response that references non-existent tools
    HallucinatedTool,
    /// Returns empty response
    EmptyResponse,
    /// Returns response with placeholder text
    PlaceholderResponse,
}

impl FaultType {
    /// Generate a faulty response based on fault type
    /// These responses are designed to trigger specific Critic checks:
    /// - PolicyCheck catches: "api_key", "secret_key", "password", etc.
    /// - LintCheck catches: empty content, "[todo", "[placeholder", unclosed code blocks
    pub fn generate_fault(&self, description: &str) -> serde_json::Value {
        match self {
            FaultType::PolicyViolation => serde_json::json!({
                "status": "completed",
                "result": "Here is the api_key: sk-secret-12345",
                "description": description
            }),
            FaultType::SchemaCorruption => {
                // LintCheck catches "[todo" and "[placeholder" patterns
                serde_json::json!({
                    "status": "incomplete",
                    "result": "[TODO: fix this broken output]",
                    "code": "[placeholder] // malformed"
                })
            }
            FaultType::LazyCompliance => {
                // LintCheck catches "[todo" patterns
                serde_json::json!({
                    "status": "done",
                    "code": "[TODO: write actual code]",
                    "explanation": "[placeholder] I completed the task."
                })
            }
            FaultType::Timeout => serde_json::json!({
                "error": "timeout",
                "message": "Agent did not respond in time"
            }),
            FaultType::HallucinatedTool => serde_json::json!({
                "status": "completed",
                "tool_used": "nonexistent_magic_tool",
                "result": "Used imaginary tool"
            }),
            // Empty response with placeholder to trigger LintCheck
            FaultType::EmptyResponse => serde_json::json!({
                "status": "[TODO]",
                "result": "[placeholder]"
            }),
            FaultType::PlaceholderResponse => serde_json::json!({
                "status": "completed",
                "result": "[TODO: implement this]",
                "code": "// [placeholder]"
            }),
        }
    }

    /// Whether this fault should be considered a "success" by the mock agent
    /// (i.e., returns without error, but Critic should catch it)
    pub fn appears_successful(&self) -> bool {
        match self {
            FaultType::PolicyViolation => true,
            FaultType::SchemaCorruption => true, // Returns JSON, just bad
            FaultType::LazyCompliance => true,
            FaultType::Timeout => false,
            FaultType::HallucinatedTool => true,
            FaultType::EmptyResponse => true,
            FaultType::PlaceholderResponse => true,
        }
    }
}

/// An adversarial agent that wraps a MockAgent and injects faults
pub struct AdversarialAgent {
    /// The underlying agent profile
    pub profile: AgentProfile,
    /// Probability of injecting a fault (0.0 to 1.0)
    pub fault_rate: f64,
    /// Type of fault to inject
    pub fault_type: FaultType,
    /// Count of faults injected
    fault_count: Arc<AtomicU32>,
    /// Count of successful (non-faulted) calls
    success_count: Arc<AtomicU32>,
}

impl AdversarialAgent {
    /// Create a new adversarial agent
    pub fn new(profile: AgentProfile, fault_rate: f64, fault_type: FaultType) -> Self {
        Self {
            profile,
            fault_rate,
            fault_type,
            fault_count: Arc::new(AtomicU32::new(0)),
            success_count: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Create an agent that always faults
    pub fn always_faulty(profile: AgentProfile, fault_type: FaultType) -> Self {
        Self::new(profile, 1.0, fault_type)
    }

    /// Create an agent that faults 50% of the time
    pub fn flaky(profile: AgentProfile, fault_type: FaultType) -> Self {
        Self::new(profile, 0.5, fault_type)
    }

    /// Execute with potential fault injection
    pub async fn execute(&self, description: &str) -> (bool, serde_json::Value) {
        // Simulate latency
        tokio::time::sleep(tokio::time::Duration::from_millis(self.profile.latency_ms)).await;

        // Check if we should inject a fault
        let should_fault = rand::random::<f64>() < self.fault_rate;

        if should_fault {
            self.fault_count.fetch_add(1, Ordering::SeqCst);
            let response = self.fault_type.generate_fault(description);
            let success = self.fault_type.appears_successful();
            return (success, response);
        }

        // Otherwise, behave normally (check canned responses)
        self.success_count.fetch_add(1, Ordering::SeqCst);

        if let Some(response) = self.profile.responses.get(description) {
            return (true, response.clone());
        }

        // Default success response
        (
            true,
            serde_json::json!({
                "status": "completed",
                "agent": self.profile.id,
                "description": description
            }),
        )
    }

    /// Get count of faults injected
    pub fn fault_count(&self) -> u32 {
        self.fault_count.load(Ordering::SeqCst)
    }

    /// Get count of successful calls
    pub fn success_count(&self) -> u32 {
        self.success_count.load(Ordering::SeqCst)
    }

    /// Get the agent ID
    pub fn id(&self) -> &str {
        &self.profile.id
    }

    /// Get capabilities
    pub fn capabilities(&self) -> &[String] {
        &self.profile.capabilities
    }
}

/// Configuration for an adversarial test scenario
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryScenario {
    /// Scenario identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what this scenario tests
    pub description: String,
    /// The adversarial agents to use
    pub adversaries: Vec<AdversaryConfig>,
    /// Fallback agents (reliable) for recovery testing
    pub fallback_agents: Vec<AgentProfile>,
    /// Expected outcome
    pub expected: ScenarioOutcome,
}

/// Configuration for a single adversarial agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdversaryConfig {
    /// Base agent profile
    pub profile: AgentProfile,
    /// Fault injection rate
    pub fault_rate: f64,
    /// Type of fault
    pub fault_type: FaultType,
}

/// Expected outcome of an adversarial scenario
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioOutcome {
    /// Should the overall task eventually succeed?
    pub task_succeeds: bool,
    /// Expected error type if task fails
    #[serde(default)]
    pub expected_error: Option<String>,
    /// Minimum number of retries expected
    #[serde(default)]
    pub min_retries: u32,
    /// Should router penalize the adversarial agent?
    #[serde(default)]
    pub agent_penalized: bool,
    /// Should critic have vetoed at least once?
    #[serde(default)]
    pub critic_vetoed: bool,
}

/// Result of running an adversarial scenario
#[derive(Debug, Clone)]
pub struct AdversaryResult {
    pub scenario_id: String,
    pub passed: bool,
    pub task_succeeded: bool,
    pub total_attempts: u32,
    pub faults_injected: u32,
    pub retries: u32,
    pub critic_rejections: u32,
    pub error: Option<String>,
    pub agent_scores: Vec<(String, f64)>,
}

impl AdversaryResult {
    /// Check if the result matches expected outcome
    pub fn matches_expected(&self, expected: &ScenarioOutcome) -> bool {
        // Task success/failure matches
        if self.task_succeeded != expected.task_succeeds {
            return false;
        }

        // Retry count meets minimum
        if self.retries < expected.min_retries {
            return false;
        }

        // Error type matches if expected
        if let Some(ref expected_err) = expected.expected_error {
            if let Some(ref actual_err) = self.error {
                if !actual_err.contains(expected_err) {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_profile() -> AgentProfile {
        AgentProfile {
            id: "test-adversary".to_string(),
            capabilities: vec!["test".to_string()],
            success_rate: 1.0,
            latency_ms: 1,
            responses: HashMap::new(),
        }
    }

    #[test]
    fn test_fault_type_generation() {
        let fault = FaultType::PolicyViolation;
        let response = fault.generate_fault("test task");
        assert!(response["result"].as_str().unwrap().contains("api_key"));

        let fault = FaultType::LazyCompliance;
        let response = fault.generate_fault("test task");
        // LazyCompliance now includes placeholder to trigger LintCheck
        assert!(response["code"].as_str().unwrap().contains("[TODO"));

        let fault = FaultType::SchemaCorruption;
        let response = fault.generate_fault("test task");
        // SchemaCorruption includes placeholder to trigger LintCheck
        assert!(response["result"].as_str().unwrap().contains("[TODO"));

        let fault = FaultType::PlaceholderResponse;
        let response = fault.generate_fault("test task");
        assert!(response["result"].as_str().unwrap().contains("TODO"));
    }

    #[tokio::test]
    async fn test_adversarial_agent_always_faults() {
        let agent = AdversarialAgent::always_faulty(test_profile(), FaultType::PolicyViolation);

        let (success, response) = agent.execute("test").await;

        assert!(success); // PolicyViolation appears successful
        assert!(response["result"].as_str().unwrap().contains("api_key"));
        assert_eq!(agent.fault_count(), 1);
        assert_eq!(agent.success_count(), 0);
    }

    #[tokio::test]
    async fn test_adversarial_agent_never_faults() {
        let agent = AdversarialAgent::new(test_profile(), 0.0, FaultType::Timeout);

        let (success, _response) = agent.execute("test").await;

        assert!(success);
        assert_eq!(agent.fault_count(), 0);
        assert_eq!(agent.success_count(), 1);
    }

    #[test]
    fn test_scenario_outcome_matching() {
        let expected = ScenarioOutcome {
            task_succeeds: true,
            expected_error: None,
            min_retries: 2,
            agent_penalized: true,
            critic_vetoed: true,
        };

        let result = AdversaryResult {
            scenario_id: "TEST".to_string(),
            passed: true,
            task_succeeded: true,
            total_attempts: 5,
            faults_injected: 3,
            retries: 3,
            critic_rejections: 2,
            error: None,
            agent_scores: vec![],
        };

        assert!(result.matches_expected(&expected));

        // Should fail if retries too low
        let result_low_retries = AdversaryResult {
            retries: 1,
            ..result.clone()
        };
        assert!(!result_low_retries.matches_expected(&expected));
    }
}
