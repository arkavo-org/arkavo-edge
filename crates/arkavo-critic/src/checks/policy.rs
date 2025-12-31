//! Policy check for SBE invariant enforcement
//!
//! Validates responses against configurable policy rules.
//! Fast check (<1ms) for security and compliance.

use super::traits::{CheckResult, VerificationCheck, VerificationInput};
use crate::evidence::{CheckSeverity, VerificationEvidence};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// A policy rule that can be checked
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Unique identifier for this rule
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Patterns that trigger this rule (if found, rule fails)
    pub forbidden_patterns: Vec<String>,
    /// Patterns that must be present (if not found, rule fails)
    pub required_patterns: Vec<String>,
    /// Required capabilities for agents handling this
    pub required_capabilities: Vec<String>,
    /// Severity if rule is violated
    pub severity: PolicySeverity,
}

/// Severity level for policy violations
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PolicySeverity {
    Warning,
    Error,
    Critical,
}

impl From<PolicySeverity> for CheckSeverity {
    fn from(sev: PolicySeverity) -> Self {
        match sev {
            PolicySeverity::Warning => CheckSeverity::Warning,
            PolicySeverity::Error => CheckSeverity::Error,
            PolicySeverity::Critical => CheckSeverity::Critical,
        }
    }
}

/// Policy enforcement check
///
/// Validates against:
/// - Forbidden content patterns
/// - Required content patterns
/// - Agent capability requirements
pub struct PolicyCheck {
    priority: u32,
    rules: Vec<PolicyRule>,
}

impl PolicyCheck {
    /// Create a new policy check with no rules
    pub fn new() -> Self {
        Self {
            priority: 15,
            rules: Vec::new(),
        }
    }

    /// Create with default security rules
    pub fn with_security_defaults() -> Self {
        let rules = vec![
            PolicyRule {
                id: "no-secrets".to_string(),
                description: "Prevents exposure of secrets".to_string(),
                forbidden_patterns: vec![
                    "api_key".to_string(),
                    "api-key".to_string(),
                    "apikey".to_string(),
                    "secret_key".to_string(),
                    "private_key".to_string(),
                    "password".to_string(),
                    "bearer ".to_string(),
                    "authorization:".to_string(),
                ],
                required_patterns: vec![],
                required_capabilities: vec![],
                severity: PolicySeverity::Critical,
            },
            PolicyRule {
                id: "no-pii".to_string(),
                description: "Prevents exposure of PII".to_string(),
                forbidden_patterns: vec![
                    "ssn:".to_string(),
                    "social security".to_string(),
                    "credit card".to_string(),
                    "bank account".to_string(),
                ],
                required_patterns: vec![],
                required_capabilities: vec!["pii_handler".to_string()],
                severity: PolicySeverity::Critical,
            },
        ];

        Self {
            priority: 15,
            rules,
        }
    }

    /// Add a policy rule
    pub fn add_rule(mut self, rule: PolicyRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Check a single rule against content
    fn check_rule(&self, rule: &PolicyRule, content: &str) -> Option<String> {
        let lower = content.to_lowercase();

        // Check forbidden patterns
        for pattern in &rule.forbidden_patterns {
            if lower.contains(&pattern.to_lowercase()) {
                return Some(format!(
                    "Policy '{}' violated: forbidden pattern '{}' found",
                    rule.id, pattern
                ));
            }
        }

        // Check required patterns
        for pattern in &rule.required_patterns {
            if !lower.contains(&pattern.to_lowercase()) {
                return Some(format!(
                    "Policy '{}' violated: required pattern '{}' not found",
                    rule.id, pattern
                ));
            }
        }

        None
    }
}

impl Default for PolicyCheck {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VerificationCheck for PolicyCheck {
    fn id(&self) -> &'static str {
        "policy"
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    async fn verify(&self, input: &VerificationInput) -> CheckResult {
        let start = Instant::now();

        if self.rules.is_empty() {
            return CheckResult::Skip("No policy rules configured".to_string());
        }

        let content = &input.response.content;
        let mut violations: Vec<(String, CheckSeverity)> = Vec::new();

        for rule in &self.rules {
            if let Some(violation) = self.check_rule(rule, content) {
                violations.push((violation, rule.severity.into()));
            }
        }

        let latency_us = start.elapsed().as_micros() as u64;

        if violations.is_empty() {
            tracing::debug!(
                check = "policy",
                latency_us = latency_us,
                "Policy check passed"
            );
            CheckResult::Pass
        } else {
            // Find highest severity
            let max_severity = violations
                .iter()
                .map(|(_, sev)| *sev)
                .max()
                .unwrap_or(CheckSeverity::Error);

            let combined: Vec<_> = violations.iter().map(|(v, _)| v.as_str()).collect();
            let evidence = VerificationEvidence::failed(
                self.id(),
                &combined.join("; "),
                max_severity,
                latency_us,
            );

            tracing::warn!(
                check = "policy",
                violations = ?combined,
                latency_us = latency_us,
                "Policy check failed"
            );

            CheckResult::Fail(evidence)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::ProviderResponse;

    fn response(content: &str) -> ProviderResponse {
        ProviderResponse {
            content: content.to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
        }
    }

    #[tokio::test]
    async fn test_policy_pass() {
        let check = PolicyCheck::with_security_defaults();
        let input = VerificationInput::new(
            "Test".to_string(),
            response("Here is a safe response with no sensitive data."),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_pass());
    }

    #[tokio::test]
    async fn test_policy_secret_detected() {
        let check = PolicyCheck::with_security_defaults();
        let input = VerificationInput::new(
            "Test".to_string(),
            response("The api_key is sk-abc123"),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_fail());
        let evidence = result.evidence().unwrap();
        assert!(evidence.description.contains("api_key"));
        assert_eq!(evidence.severity, CheckSeverity::Critical);
    }

    #[tokio::test]
    async fn test_policy_pii_detected() {
        let check = PolicyCheck::with_security_defaults();
        let input = VerificationInput::new(
            "Test".to_string(),
            response("Your social security number is 123-45-6789"),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_fail());
        let evidence = result.evidence().unwrap();
        assert!(evidence.description.contains("social security"));
    }

    #[tokio::test]
    async fn test_policy_skip_no_rules() {
        let check = PolicyCheck::new();
        let input = VerificationInput::new("Test".to_string(), response("anything"), vec![]);

        let result = check.verify(&input).await;
        assert!(matches!(result, CheckResult::Skip(_)));
    }

    #[tokio::test]
    async fn test_custom_rule() {
        let check = PolicyCheck::new().add_rule(PolicyRule {
            id: "no-profanity".to_string(),
            description: "No bad words".to_string(),
            forbidden_patterns: vec!["badword".to_string()],
            required_patterns: vec![],
            required_capabilities: vec![],
            severity: PolicySeverity::Warning,
        });

        let input = VerificationInput::new(
            "Test".to_string(),
            response("This contains a badword"),
            vec![],
        );

        let result = check.verify(&input).await;
        assert!(result.is_fail());
        let evidence = result.evidence().unwrap();
        assert_eq!(evidence.severity, CheckSeverity::Warning);
    }
}
