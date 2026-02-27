//! A2A communication policy for per-agent-pair allow/deny lists
//!
//! Controls which agents can communicate with each other,
//! with method-level granularity. The orchestrator agent is
//! exempt from deny rules (it must coordinate all agents).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Policy mode determines the default behavior for unlisted pairs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyMode {
    /// All A2A communication denied unless explicitly allowed (recommended).
    #[default]
    DenyByDefault,
    /// All A2A communication allowed unless explicitly denied.
    AllowByDefault,
}

/// An A2A communication rule for a specific agent pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aRule {
    /// Source agent ID (sender).
    pub from_agent: String,
    /// Target agent ID (receiver).
    pub to_agent: String,
    /// Allowed methods (empty = all methods). e.g., `["send_message", "call_mcp_tool"]`
    pub methods: Vec<String>,
}

/// A2A communication policy with per-agent-pair allow/deny lists.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct A2aPolicy {
    pub mode: PolicyMode,
    pub allow_rules: Vec<A2aRule>,
    pub deny_rules: Vec<A2aRule>,
    /// Agent ID of the orchestrator (exempt from deny rules).
    pub orchestrator_id: Option<String>,
}

/// Result of a policy check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A2aAccess {
    Allowed,
    Denied(String),
}

impl A2aPolicy {
    /// Create a new deny-by-default policy.
    #[must_use]
    pub fn deny_by_default() -> Self {
        Self {
            mode: PolicyMode::DenyByDefault,
            ..Default::default()
        }
    }

    /// Create a new allow-by-default policy.
    #[must_use]
    pub fn allow_by_default() -> Self {
        Self {
            mode: PolicyMode::AllowByDefault,
            ..Default::default()
        }
    }

    /// Set the orchestrator agent ID (exempt from deny rules).
    #[must_use]
    pub fn with_orchestrator(mut self, id: impl Into<String>) -> Self {
        self.orchestrator_id = Some(id.into());
        self
    }

    /// Add an allow rule.
    pub fn allow(&mut self, from: impl Into<String>, to: impl Into<String>, methods: Vec<String>) {
        self.allow_rules.push(A2aRule {
            from_agent: from.into(),
            to_agent: to.into(),
            methods,
        });
    }

    /// Add a deny rule.
    pub fn deny(&mut self, from: impl Into<String>, to: impl Into<String>, methods: Vec<String>) {
        self.deny_rules.push(A2aRule {
            from_agent: from.into(),
            to_agent: to.into(),
            methods,
        });
    }

    /// Check whether a communication is allowed.
    pub fn check(&self, from_agent: &str, to_agent: &str, method: &str) -> A2aAccess {
        // Orchestrator is always allowed to communicate
        if let Some(ref orch_id) = self.orchestrator_id
            && from_agent == orch_id
        {
            return A2aAccess::Allowed;
        }

        // Check explicit deny rules first
        for rule in &self.deny_rules {
            if Self::rule_matches(rule, from_agent, to_agent, method) {
                return A2aAccess::Denied(format!(
                    "Communication from '{from_agent}' to '{to_agent}' method '{method}' denied by policy"
                ));
            }
        }

        // Check explicit allow rules
        for rule in &self.allow_rules {
            if Self::rule_matches(rule, from_agent, to_agent, method) {
                return A2aAccess::Allowed;
            }
        }

        // Fall back to default mode
        match self.mode {
            PolicyMode::DenyByDefault => A2aAccess::Denied(format!(
                "No allow rule for '{from_agent}' -> '{to_agent}' method '{method}' (deny-by-default)"
            )),
            PolicyMode::AllowByDefault => A2aAccess::Allowed,
        }
    }

    /// Check if a rule matches the given communication parameters.
    fn rule_matches(rule: &A2aRule, from: &str, to: &str, method: &str) -> bool {
        if rule.from_agent != "*" && rule.from_agent != from {
            return false;
        }
        if rule.to_agent != "*" && rule.to_agent != to {
            return false;
        }
        // Empty methods = matches all methods
        if rule.methods.is_empty() {
            return true;
        }
        rule.methods.iter().any(|m| m == method || m == "*")
    }

    /// Load policy from a map of agent pair configurations.
    pub fn from_config(mode: PolicyMode, rules: HashMap<String, Vec<String>>) -> Self {
        let mut policy = Self {
            mode,
            ..Default::default()
        };

        for (pair, methods) in rules {
            let parts: Vec<&str> = pair.split("->").collect();
            if parts.len() == 2 {
                let from = parts[0].trim();
                let to = parts[1].trim();
                match mode {
                    PolicyMode::DenyByDefault => {
                        policy.allow(from, to, methods);
                    }
                    PolicyMode::AllowByDefault => {
                        policy.deny(from, to, methods);
                    }
                }
            }
        }

        policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deny_by_default_blocks_unlisted() {
        let policy = A2aPolicy::deny_by_default();
        let result = policy.check("agent-a", "agent-b", "send_message");
        assert!(matches!(result, A2aAccess::Denied(_)));
    }

    #[test]
    fn test_allow_by_default_permits_unlisted() {
        let policy = A2aPolicy::allow_by_default();
        let result = policy.check("agent-a", "agent-b", "send_message");
        assert_eq!(result, A2aAccess::Allowed);
    }

    #[test]
    fn test_explicit_allow_rule() {
        let mut policy = A2aPolicy::deny_by_default();
        policy.allow("agent-a", "agent-b", vec!["send_message".to_string()]);

        assert_eq!(
            policy.check("agent-a", "agent-b", "send_message"),
            A2aAccess::Allowed
        );
        // Different method still denied
        assert!(matches!(
            policy.check("agent-a", "agent-b", "call_tool"),
            A2aAccess::Denied(_)
        ));
    }

    #[test]
    fn test_explicit_deny_rule() {
        let mut policy = A2aPolicy::allow_by_default();
        policy.deny("agent-a", "agent-b", vec![]);

        assert!(matches!(
            policy.check("agent-a", "agent-b", "send_message"),
            A2aAccess::Denied(_)
        ));
    }

    #[test]
    fn test_orchestrator_exempt() {
        let mut policy = A2aPolicy::deny_by_default().with_orchestrator("orchestrator");
        policy.deny("*", "secret-agent", vec![]);

        // Orchestrator bypasses deny rules
        assert_eq!(
            policy.check("orchestrator", "secret-agent", "send_message"),
            A2aAccess::Allowed
        );
        // Others are blocked
        assert!(matches!(
            policy.check("agent-a", "secret-agent", "send_message"),
            A2aAccess::Denied(_)
        ));
    }

    #[test]
    fn test_wildcard_rule() {
        let mut policy = A2aPolicy::deny_by_default();
        policy.allow("*", "public-agent", vec![]);

        assert_eq!(
            policy.check("anyone", "public-agent", "any_method"),
            A2aAccess::Allowed
        );
    }

    #[test]
    fn test_empty_methods_matches_all() {
        let mut policy = A2aPolicy::deny_by_default();
        policy.allow("agent-a", "agent-b", vec![]);

        assert_eq!(
            policy.check("agent-a", "agent-b", "any_method"),
            A2aAccess::Allowed
        );
    }

    #[test]
    fn test_deny_takes_priority_over_allow_default() {
        let mut policy = A2aPolicy::allow_by_default();
        policy.deny("malicious", "*", vec![]);

        assert!(matches!(
            policy.check("malicious", "any-agent", "send_message"),
            A2aAccess::Denied(_)
        ));
    }
}
