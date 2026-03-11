//! Autonomous policy decision point for tool execution
//!
//! The Task Policy Manager composes entitlement checks, budget gates,
//! and safety invariants to make autonomous Permit/Deny decisions.
//! No human-in-the-loop approval — the orchestrator agent is the decision-maker.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Context passed to the policy manager for each tool execution decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskContext {
    pub task_id: Uuid,
    pub agent_id: String,
    pub entitlements: Vec<String>,
    pub tool_name: String,
    pub tool_params: serde_json::Value,
    pub estimated_cost_dollars: f64,
    pub spawn_depth: u32,
    pub task_status: String,
}

/// Autonomous policy decision — richer than binary Permit/Deny.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyDecision {
    Permit,
    PermitWithObligations(Vec<Obligation>),
    Deny(String),
}

/// Obligations attached to a permitted action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Obligation {
    /// Execute in a sandboxed environment.
    Sandbox(SandboxConfig),
    /// Log execution details for audit trail.
    AuditLog,
    /// Debit the specified cost from budget after execution.
    BudgetDebit(f64),
    /// Report execution result back to orchestrator.
    NotifyOrchestrator,
}

/// Sandbox configuration for obligated executions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub use_docker: bool,
    pub network_access: bool,
    pub filesystem_readonly: bool,
    pub timeout_secs: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            use_docker: true,
            network_access: false,
            filesystem_readonly: true,
            timeout_secs: 60,
        }
    }
}

/// Tool risk classification for obligation assembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRisk {
    /// Read-only operations: ls, git status, etc.
    Low,
    /// Build, test, package install.
    Medium,
    /// Shell exec, file writes, network access.
    High,
    /// System modification, privilege escalation.
    Critical,
}

/// Entitlement requirement for a tool.
#[derive(Debug, Clone)]
pub struct ToolEntitlement {
    pub tool_name: String,
    pub required_entitlements: Vec<String>,
    pub risk: ToolRisk,
}

/// Task Policy Manager — autonomous decision point for tool execution.
///
/// Composes three checks:
/// 1. Entitlement check: agent has required entitlements for the tool
/// 2. Budget check: agent can afford the estimated cost
/// 3. Invariant check: action doesn't violate hard safety constraints
pub struct TaskPolicyManager {
    tool_requirements: Vec<ToolEntitlement>,
    budget_limit: f64,
    max_spawn_depth: u32,
    blocked_tools: Vec<String>,
}

impl TaskPolicyManager {
    /// Create a new policy manager with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_requirements: Vec::new(),
            budget_limit: f64::MAX,
            max_spawn_depth: 1,
            blocked_tools: Vec::new(),
        }
    }

    /// Set the budget limit for cost checks.
    #[must_use]
    pub fn with_budget_limit(mut self, limit: f64) -> Self {
        self.budget_limit = limit;
        self
    }

    /// Set the max spawn depth.
    #[must_use]
    pub fn with_max_spawn_depth(mut self, depth: u32) -> Self {
        self.max_spawn_depth = depth;
        self
    }

    /// Register entitlement requirements for a tool.
    pub fn register_tool_requirement(&mut self, requirement: ToolEntitlement) {
        self.tool_requirements.push(requirement);
    }

    /// Block a tool from execution entirely.
    pub fn block_tool(&mut self, tool_name: impl Into<String>) {
        self.blocked_tools.push(tool_name.into());
    }

    /// Evaluate policy for a tool execution request.
    ///
    /// Decision flow:
    /// 1. Check if tool is explicitly blocked
    /// 2. Verify spawn depth limits
    /// 3. Verify agent entitlements satisfy tool requirements
    /// 4. Check budget can afford estimated cost
    /// 5. Assemble obligations based on risk classification
    pub fn evaluate(&self, ctx: &TaskContext) -> PolicyDecision {
        // Step 1: Blocked tool check
        if self.blocked_tools.contains(&ctx.tool_name) {
            return PolicyDecision::Deny(format!(
                "Tool '{}' is explicitly blocked by policy",
                ctx.tool_name
            ));
        }

        // Step 2: Spawn depth check
        if ctx.spawn_depth > self.max_spawn_depth {
            return PolicyDecision::Deny(format!(
                "Spawn depth {} exceeds maximum {}",
                ctx.spawn_depth, self.max_spawn_depth
            ));
        }

        // Step 3: Entitlement check
        let tool_req = self
            .tool_requirements
            .iter()
            .find(|r| r.tool_name == ctx.tool_name);

        let risk = if let Some(req) = tool_req {
            for required in &req.required_entitlements {
                if !ctx.entitlements.contains(required) {
                    return PolicyDecision::Deny(format!(
                        "Agent '{}' lacks entitlement '{}' required for tool '{}'",
                        ctx.agent_id, required, ctx.tool_name
                    ));
                }
            }
            req.risk
        } else {
            ToolRisk::Medium
        };

        // Step 4: Budget check
        if ctx.estimated_cost_dollars > self.budget_limit {
            return PolicyDecision::Deny(format!(
                "Estimated cost ${:.4} exceeds budget limit ${:.4}",
                ctx.estimated_cost_dollars, self.budget_limit
            ));
        }

        // Step 5: Assemble obligations based on risk
        let obligations = self.assemble_obligations(risk, ctx);

        if obligations.is_empty() {
            PolicyDecision::Permit
        } else {
            PolicyDecision::PermitWithObligations(obligations)
        }
    }

    /// Assemble execution obligations based on tool risk level.
    fn assemble_obligations(&self, risk: ToolRisk, ctx: &TaskContext) -> Vec<Obligation> {
        let mut obligations = Vec::new();

        match risk {
            ToolRisk::Low => {
                // No obligations for low-risk operations
            }
            ToolRisk::Medium => {
                obligations.push(Obligation::AuditLog);
                if ctx.estimated_cost_dollars > 0.0 {
                    obligations.push(Obligation::BudgetDebit(ctx.estimated_cost_dollars));
                }
            }
            ToolRisk::High => {
                obligations.push(Obligation::AuditLog);
                obligations.push(Obligation::NotifyOrchestrator);
                obligations.push(Obligation::Sandbox(SandboxConfig::default()));
                if ctx.estimated_cost_dollars > 0.0 {
                    obligations.push(Obligation::BudgetDebit(ctx.estimated_cost_dollars));
                }
            }
            ToolRisk::Critical => {
                obligations.push(Obligation::AuditLog);
                obligations.push(Obligation::NotifyOrchestrator);
                obligations.push(Obligation::Sandbox(SandboxConfig {
                    use_docker: true,
                    network_access: false,
                    filesystem_readonly: true,
                    timeout_secs: 30,
                }));
                if ctx.estimated_cost_dollars > 0.0 {
                    obligations.push(Obligation::BudgetDebit(ctx.estimated_cost_dollars));
                }
            }
        }

        obligations
    }
}

impl Default for TaskPolicyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PolicyDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permit => write!(f, "Permit"),
            Self::PermitWithObligations(obs) => {
                write!(f, "Permit with {} obligation(s)", obs.len())
            }
            Self::Deny(reason) => write!(f, "Deny: {}", reason),
        }
    }
}

impl PolicyDecision {
    /// Whether the decision allows execution.
    #[must_use]
    pub fn is_permitted(&self) -> bool {
        matches!(self, Self::Permit | Self::PermitWithObligations(_))
    }

    /// Whether the decision denies execution.
    #[must_use]
    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Deny(_))
    }

    /// Get obligations if any.
    #[must_use]
    pub fn obligations(&self) -> &[Obligation] {
        match self {
            Self::PermitWithObligations(obs) => obs,
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context(tool: &str) -> TaskContext {
        TaskContext {
            task_id: Uuid::new_v4(),
            agent_id: "test-agent".to_string(),
            entitlements: vec![
                "https://arkavo.net/attr/role/value/developer".to_string(),
                "https://arkavo.net/attr/tool/value/shell_exec".to_string(),
            ],
            tool_name: tool.to_string(),
            tool_params: serde_json::json!({}),
            estimated_cost_dollars: 0.001,
            spawn_depth: 0,
            task_status: "working".to_string(),
        }
    }

    #[test]
    fn test_permit_no_requirements() {
        let pm = TaskPolicyManager::new();
        let ctx = test_context("ls");
        let decision = pm.evaluate(&ctx);
        assert!(decision.is_permitted());
    }

    #[test]
    fn test_deny_blocked_tool() {
        let mut pm = TaskPolicyManager::new();
        pm.block_tool("dangerous_tool");
        let ctx = test_context("dangerous_tool");
        let decision = pm.evaluate(&ctx);
        assert!(decision.is_denied());
        if let PolicyDecision::Deny(reason) = decision {
            assert!(reason.contains("blocked"));
        }
    }

    #[test]
    fn test_deny_missing_entitlement() {
        let mut pm = TaskPolicyManager::new();
        pm.register_tool_requirement(ToolEntitlement {
            tool_name: "shell_exec".to_string(),
            required_entitlements: vec!["https://arkavo.net/attr/role/value/admin".to_string()],
            risk: ToolRisk::High,
        });
        let ctx = test_context("shell_exec");
        let decision = pm.evaluate(&ctx);
        assert!(decision.is_denied());
    }

    #[test]
    fn test_permit_with_entitlements() {
        let mut pm = TaskPolicyManager::new();
        pm.register_tool_requirement(ToolEntitlement {
            tool_name: "shell_exec".to_string(),
            required_entitlements: vec![
                "https://arkavo.net/attr/tool/value/shell_exec".to_string(),
            ],
            risk: ToolRisk::High,
        });
        let ctx = test_context("shell_exec");
        let decision = pm.evaluate(&ctx);
        assert!(decision.is_permitted());
        // High risk → should have sandbox obligation
        let obs = decision.obligations();
        assert!(obs.iter().any(|o| matches!(o, Obligation::Sandbox(_))));
    }

    #[test]
    fn test_deny_over_budget() {
        let pm = TaskPolicyManager::new().with_budget_limit(0.0001);
        let ctx = test_context("expensive_tool");
        let decision = pm.evaluate(&ctx);
        assert!(decision.is_denied());
    }

    #[test]
    fn test_deny_spawn_depth_exceeded() {
        let pm = TaskPolicyManager::new().with_max_spawn_depth(1);
        let mut ctx = test_context("tool");
        ctx.spawn_depth = 5;
        let decision = pm.evaluate(&ctx);
        assert!(decision.is_denied());
    }

    #[test]
    fn test_low_risk_no_obligations() {
        let mut pm = TaskPolicyManager::new();
        pm.register_tool_requirement(ToolEntitlement {
            tool_name: "ls".to_string(),
            required_entitlements: vec![],
            risk: ToolRisk::Low,
        });
        let mut ctx = test_context("ls");
        ctx.estimated_cost_dollars = 0.0;
        let decision = pm.evaluate(&ctx);
        assert!(matches!(decision, PolicyDecision::Permit));
    }

    #[test]
    fn test_medium_risk_audit_obligation() {
        let mut pm = TaskPolicyManager::new();
        pm.register_tool_requirement(ToolEntitlement {
            tool_name: "cargo_build".to_string(),
            required_entitlements: vec![],
            risk: ToolRisk::Medium,
        });
        let ctx = test_context("cargo_build");
        let decision = pm.evaluate(&ctx);
        let obs = decision.obligations();
        assert!(obs.iter().any(|o| matches!(o, Obligation::AuditLog)));
    }

    #[test]
    fn test_critical_risk_all_obligations() {
        let mut pm = TaskPolicyManager::new();
        pm.register_tool_requirement(ToolEntitlement {
            tool_name: "system_modify".to_string(),
            required_entitlements: vec![],
            risk: ToolRisk::Critical,
        });
        let ctx = test_context("system_modify");
        let decision = pm.evaluate(&ctx);
        let obs = decision.obligations();
        assert!(obs.iter().any(|o| matches!(o, Obligation::Sandbox(_))));
        assert!(obs.iter().any(|o| matches!(o, Obligation::AuditLog)));
        assert!(
            obs.iter()
                .any(|o| matches!(o, Obligation::NotifyOrchestrator))
        );
    }

    #[test]
    fn test_display_decision() {
        assert_eq!(format!("{}", PolicyDecision::Permit), "Permit");
        assert_eq!(
            format!("{}", PolicyDecision::Deny("reason".into())),
            "Deny: reason"
        );
    }
}
