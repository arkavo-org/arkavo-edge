use crate::ai::claude_client::ClaudeClient;
use crate::mcp::tools::ToolRegistry;
use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestPlan {
    pub objectives: Vec<String>,
    pub duration_minutes: u32,
    pub strategies: Vec<TestStrategy>,
    pub invariants: Vec<PropertyInvariant>,
    pub chaos_scenarios: Vec<ChaosScenario>,
    pub benchmarks: Vec<PerformanceBenchmark>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestStrategy {
    pub name: String,
    pub description: String,
    pub steps: Vec<TestStep>,
    pub priority: Priority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestStep {
    pub action: String,
    pub expected_outcome: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyInvariant {
    pub name: String,
    pub description: String,
    pub check_expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosScenario {
    pub name: String,
    pub description: String,
    pub fault_injection: FaultType,
    pub expected_behavior: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultType {
    NetworkLatency { ms: u32 },
    NetworkPacketLoss { percent: u8 },
    ResourceExhaustion { resource: String },
    ConcurrentRequests { count: u32 },
    DataCorruption { target: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceBenchmark {
    pub name: String,
    pub operation: String,
    pub success_criteria: BenchmarkCriteria,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkCriteria {
    pub max_duration_ms: u32,
    pub max_memory_mb: u32,
    pub min_throughput: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub description: String,
    pub reproduction_steps: Vec<String>,
    pub minimal_test: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

pub struct TestPlanner {
    claude: ClaudeClient,
    tool_registry: Arc<ToolRegistry>,
    state_tracker: StateTracker,
}

impl TestPlanner {
    pub fn new(claude: ClaudeClient, tool_registry: Arc<ToolRegistry>) -> Self {
        Self {
            claude,
            tool_registry,
            state_tracker: StateTracker::new(),
        }
    }

    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.tool_registry
    }

    pub async fn plan_test_session(
        &self,
        objectives: Vec<String>,
        duration_minutes: u32,
    ) -> Result<TestPlan> {
        let state = self.get_initial_state().await?;

        let strategy_prompt = format!(
            r#"Given the current application state and test objectives, generate a comprehensive test plan.

Application State:
{}

Test Objectives:
{}

Duration: {} minutes

Generate a test plan with:
1. Property invariants to check (things that should always be true)
2. State transitions to explore
3. Chaos scenarios to inject (network issues, resource constraints, etc.)
4. Performance benchmarks

Return a JSON object with:
- strategies: Array of test strategies with steps
- invariants: Array of property invariants
- chaos_scenarios: Array of fault injection scenarios
- benchmarks: Array of performance criteria

Example format:
{{
  "strategies": [
    {{
      "name": "User Authentication Flow",
      "description": "Test login, logout, and session management",
      "steps": [
        {{
          "action": "Login with valid credentials",
          "expected_outcome": "User is authenticated and redirected",
          "tools": ["mutate_state", "query_state"]
        }}
      ],
      "priority": "critical"
    }}
  ],
  "invariants": [
    {{
      "name": "Account Balance Non-negative",
      "description": "User account balance should never go below zero",
      "check_expression": "account.balance >= 0"
    }}
  ],
  "chaos_scenarios": [
    {{
      "name": "Network Latency During Transaction",
      "description": "Inject 500ms latency during payment processing",
      "fault_injection": {{
        "network_latency": {{ "ms": 500 }}
      }},
      "expected_behavior": "Transaction completes within timeout or fails gracefully"
    }}
  ],
  "benchmarks": [
    {{
      "name": "Login Performance",
      "operation": "user_login",
      "success_criteria": {{
        "max_duration_ms": 200,
        "max_memory_mb": 50
      }}
    }}
  ]
}}"#,
            serde_json::to_string_pretty(&state).unwrap_or_default(),
            objectives
                .iter()
                .map(|o| format!("- {}", o))
                .collect::<Vec<_>>()
                .join("\n"),
            duration_minutes
        );

        let response = self.claude.complete(&strategy_prompt).await?;

        self.parse_test_plan(&response, objectives, duration_minutes)
    }

    pub async fn adaptive_exploration(&mut self) -> Result<Vec<Finding>> {
        let mut findings = Vec::new();
        let max_iterations = 100;

        for _iteration in 0..max_iterations {
            let current_state = self.get_current_state().await?;
            let history = self.state_tracker.get_recent_actions(10);

            let exploration_prompt = format!(
                r#"You are exploring an application to find issues. Based on the current state and recent actions, decide the next action.

Current State:
{}

Recent Actions:
{}

Previous Findings:
{}

What action should we take next? Consider:
1. Unexplored areas of the application
2. Edge cases and boundary conditions
3. Potential security vulnerabilities
4. Performance bottlenecks
5. Data integrity issues

Return a JSON object with:
- action: The tool to call
- parameters: Parameters for the tool
- hypothesis: What you're testing
- stop_exploration: true if we should stop exploring

Example:
{{
  "action": "mutate_state",
  "parameters": {{
    "entity": "user_account",
    "action": "withdraw",
    "data": {{ "amount": -100 }}
  }},
  "hypothesis": "Testing negative withdrawal amount handling",
  "stop_exploration": false
}}"#,
                serde_json::to_string_pretty(&current_state).unwrap_or_default(),
                serde_json::to_string_pretty(&history).unwrap_or_default(),
                serde_json::to_string_pretty(&findings).unwrap_or_default()
            );

            let response = self.claude.complete(&exploration_prompt).await?;
            let decision: ExplorationDecision = serde_json::from_str(&response).map_err(|e| {
                TestError::Ai(format!("Failed to parse exploration decision: {}", e))
            })?;

            if decision.stop_exploration {
                break;
            }

            let action = decision.action.clone();
            let params = decision.parameters.clone();

            match self.execute_action(&action, params.clone()).await {
                Ok(result) => {
                    if let Some(issue) = self.detect_issue(&result, &decision.hypothesis).await? {
                        let minimized = self.minimize_reproduction(issue).await?;
                        findings.push(minimized);
                    }
                }
                Err(e) => {
                    findings.push(Finding {
                        severity: Severity::High,
                        title: format!("Error during {}", action),
                        description: format!("Action failed: {}", e),
                        reproduction_steps: vec![format!(
                            "Execute {} with parameters: {:?}",
                            action, params
                        )],
                        minimal_test: None,
                        timestamp: chrono::Utc::now(),
                    });
                }
            }

            self.state_tracker.record_action(action, params);
        }

        Ok(findings)
    }

    async fn get_initial_state(&self) -> Result<Value> {
        Ok(serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "environment": "test",
            "version": "1.0.0"
        }))
    }

    async fn get_current_state(&self) -> Result<Value> {
        Ok(serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "state": "current"
        }))
    }

    async fn execute_action(&self, action: &str, params: Value) -> Result<Value> {
        Ok(serde_json::json!({
            "action": action,
            "params": params,
            "result": "success"
        }))
    }

    async fn detect_issue(&self, _result: &Value, _hypothesis: &str) -> Result<Option<Finding>> {
        Ok(None)
    }

    async fn minimize_reproduction(&self, finding: Finding) -> Result<Finding> {
        Ok(finding)
    }

    fn parse_test_plan(
        &self,
        response: &str,
        objectives: Vec<String>,
        duration_minutes: u32,
    ) -> Result<TestPlan> {
        let parsed: serde_json::Value = serde_json::from_str(response)
            .map_err(|e| TestError::Ai(format!("Failed to parse test plan: {}", e)))?;

        let strategies = parsed
            .get("strategies")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<TestStrategy>(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        let invariants = parsed
            .get("invariants")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<PropertyInvariant>(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        let chaos_scenarios = parsed
            .get("chaos_scenarios")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<ChaosScenario>(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        let benchmarks = parsed
            .get("benchmarks")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<PerformanceBenchmark>(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(TestPlan {
            objectives,
            duration_minutes,
            strategies,
            invariants,
            chaos_scenarios,
            benchmarks,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExplorationDecision {
    action: String,
    parameters: Value,
    hypothesis: String,
    stop_exploration: bool,
}

pub struct StateTracker {
    actions: std::sync::RwLock<Vec<(String, Value)>>,
}

impl StateTracker {
    pub fn new() -> Self {
        Self {
            actions: std::sync::RwLock::new(Vec::new()),
        }
    }

    pub fn record_action(&self, action: String, params: Value) {
        if let Ok(mut actions) = self.actions.write() {
            actions.push((action, params));
            if actions.len() > 1000 {
                actions.drain(0..100);
            }
        }
    }

    pub fn get_recent_actions(&self, count: usize) -> Vec<(String, Value)> {
        if let Ok(actions) = self.actions.read() {
            actions
                .iter()
                .rev()
                .take(count)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        } else {
            Vec::new()
        }
    }
}

impl Default for StateTracker {
    fn default() -> Self {
        Self::new()
    }
}
