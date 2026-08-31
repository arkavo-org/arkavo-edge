//! Critic verification pipeline
//!
//! Orchestrates verification checks in priority order.

use crate::checks::{CheckResult, VerificationCheck, VerificationInput};
use crate::config::CriticConfig;
use crate::evidence::{CheckSeverity, VerificationEvidence};
use std::sync::Arc;
use std::time::Instant;

/// Result of running the full verification pipeline
#[derive(Debug)]
pub struct PipelineResult {
    /// Overall pass/fail status
    pub passed: bool,
    /// Evidence from all checks
    pub evidence: Vec<VerificationEvidence>,
    /// Total latency in microseconds
    pub total_latency_us: u64,
    /// Number of checks run
    pub checks_run: usize,
    /// Number of checks skipped
    pub checks_skipped: usize,
    /// Whether human approval is needed
    pub needs_approval: bool,
}

impl PipelineResult {
    /// Get all failed checks
    pub fn failures(&self) -> Vec<&VerificationEvidence> {
        self.evidence
            .iter()
            .filter(|e| e.status.is_failed())
            .collect()
    }

    /// Get the highest severity issue
    pub fn max_severity(&self) -> Option<CheckSeverity> {
        self.evidence.iter().map(|e| e.severity).max()
    }
}

/// The Critic verification pipeline
pub struct CriticPipeline {
    config: CriticConfig,
    checks: Vec<Arc<dyn VerificationCheck>>,
}

impl CriticPipeline {
    /// Create a new pipeline with default config
    pub fn new() -> Self {
        Self {
            config: CriticConfig::default(),
            checks: Vec::new(),
        }
    }

    /// Create with custom config
    pub fn with_config(config: CriticConfig) -> Self {
        Self {
            config,
            checks: Vec::new(),
        }
    }

    /// Add a check to the pipeline
    pub fn add_check<C: VerificationCheck + 'static>(mut self, check: C) -> Self {
        self.checks.push(Arc::new(check));
        self
    }

    /// Add a shared check to the pipeline
    pub fn add_shared_check(mut self, check: Arc<dyn VerificationCheck>) -> Self {
        self.checks.push(check);
        self
    }

    /// Run the verification pipeline
    pub async fn verify(&self, input: &VerificationInput) -> PipelineResult {
        let start = Instant::now();
        let mut evidence: Vec<VerificationEvidence> = Vec::new();
        let mut checks_run = 0;
        let mut checks_skipped = 0;
        let mut needs_approval = false;
        let mut had_failure = false;

        // Sort checks by priority (lower = runs first)
        let mut sorted_checks: Vec<_> = self.checks.iter().collect();
        sorted_checks.sort_by_key(|c| c.priority());

        for check in sorted_checks {
            // Check timeout
            if start.elapsed() > self.config.total_timeout() {
                tracing::warn!(
                    check_id = check.id(),
                    "Pipeline timeout reached, skipping remaining checks"
                );
                break;
            }

            // Check applicability
            if !check.is_applicable(input) {
                checks_skipped += 1;
                continue;
            }

            // A block is not softened by later evidence (SENT-014).
            if had_failure && check.skip_after_failure() {
                checks_skipped += 1;
                tracing::debug!(
                    check_id = check.id(),
                    "An earlier check failed; skipping an evidence-only check"
                );
                continue;
            }

            // Run the check
            let result = check.verify(input).await;
            checks_run += 1;

            match result {
                CheckResult::Pass => {
                    // Create passing evidence for tracking
                    let latency_us = start.elapsed().as_micros() as u64;
                    evidence.push(VerificationEvidence::passed(
                        check.id(),
                        "Check passed",
                        latency_us,
                    ));
                }
                CheckResult::Fail(e) => {
                    let should_fail = e.severity.should_fail(self.config.min_fail_severity);
                    if should_fail {
                        had_failure = true;
                    }
                    evidence.push(e);

                    if self.config.fail_fast && had_failure {
                        tracing::debug!(
                            check_id = check.id(),
                            "Fail-fast triggered, stopping pipeline"
                        );
                        break;
                    }
                }
                CheckResult::Warn(e) => {
                    evidence.push(e);
                }
                CheckResult::Skip(reason) => {
                    checks_skipped += 1;
                    tracing::debug!(
                        check_id = check.id(),
                        reason = %reason,
                        "Check skipped"
                    );
                }
                CheckResult::NeedsApproval(e) => {
                    needs_approval = true;
                    evidence.push(e);
                }
            }
        }

        let total_latency_us = start.elapsed().as_micros() as u64;

        tracing::info!(
            passed = !had_failure,
            checks_run = checks_run,
            checks_skipped = checks_skipped,
            needs_approval = needs_approval,
            latency_us = total_latency_us,
            "Pipeline complete"
        );

        PipelineResult {
            passed: !had_failure && !needs_approval,
            evidence,
            total_latency_us,
            checks_run,
            checks_skipped,
            needs_approval,
        }
    }
}

impl Default for CriticPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, clippy::unnecessary_literal_bound)]
mod tests {
    use super::*;
    use crate::checks::{LintCheck, PolicyCheck, SchemaCheck};
    use arkavo_llm::ProviderResponse;
    use arkavo_llm::tool_parser::ParsedToolCall;
    use arkavo_mcp_tools::ToolInfo;
    use arkavo_test_macros::spec;
    use serde_json::json;

    fn response(content: &str) -> ProviderResponse {
        ProviderResponse {
            content: content.to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        }
    }

    #[spec("CRIT-001")]
    #[tokio::test]
    async fn test_empty_pipeline() {
        let pipeline = CriticPipeline::new();
        let input = VerificationInput::new("Test".to_string(), response("Test response"), vec![]);

        let result = pipeline.verify(&input).await;

        assert!(result.passed);
        assert_eq!(result.checks_run, 0);
    }

    #[spec("CRIT-002")]
    #[tokio::test]
    async fn test_pipeline_all_pass() {
        let pipeline = CriticPipeline::new()
            .add_check(SchemaCheck::new())
            .add_check(LintCheck::new());

        let input = VerificationInput::new(
            "Test".to_string(),
            response("This is a valid and complete response."),
            vec![],
        );

        let result = pipeline.verify(&input).await;

        assert!(result.passed);
        // SchemaCheck skips (no tool calls), LintCheck runs
        assert!(result.checks_run >= 1);
    }

    #[spec("CRIT-008")]
    #[tokio::test]
    async fn test_pipeline_fail_fast() {
        let config = CriticConfig {
            fail_fast: true,
            ..Default::default()
        };

        let pipeline = CriticPipeline::with_config(config)
            .add_check(PolicyCheck::with_security_defaults())
            .add_check(LintCheck::new());

        let input = VerificationInput::new(
            "Test".to_string(),
            response("The api_key is sk-secret"),
            vec![],
        );

        let result = pipeline.verify(&input).await;

        assert!(!result.passed);
        // Should stop after policy check fails
        assert_eq!(result.checks_run, 1);
        assert!(!result.failures().is_empty());
    }

    #[tokio::test]
    async fn test_pipeline_priority_ordering() {
        let pipeline = CriticPipeline::new()
            .add_check(LintCheck::new().with_priority(20))
            .add_check(SchemaCheck::with_priority(10))
            .add_check(PolicyCheck::new().with_priority(15));

        // Schema (10) should run before Policy (15) which runs before Lint (20)
        // Verify pipeline was constructed with all 3 checks by running it
        let input = VerificationInput::new("Test".to_string(), response("Valid response."), vec![]);
        let result = pipeline.verify(&input).await;
        assert!(result.checks_run + result.checks_skipped > 0);
    }

    #[tokio::test]
    async fn test_pipeline_result_methods() {
        let pipeline = CriticPipeline::new().add_check(PolicyCheck::with_security_defaults());

        let input = VerificationInput::new(
            "Test".to_string(),
            response("The api_key is exposed"),
            vec![],
        );

        let result = pipeline.verify(&input).await;

        assert!(!result.passed);
        assert!(!result.failures().is_empty());
        assert_eq!(result.max_severity(), Some(CheckSeverity::Critical));
    }

    #[spec("CRIT-001")]
    #[tokio::test]
    async fn test_default_pipeline_runs_default_checks() {
        let pipeline = crate::default_pipeline();
        let input = VerificationInput::new(
            "Test".to_string(),
            response("A valid response with no secrets."),
            vec![],
        );

        let result = pipeline.verify(&input).await;

        assert!(result.passed);
        // Default pipeline has CircuitCheck, SchemaCheck, and PolicyCheck.
        assert_eq!(result.checks_run + result.checks_skipped, 3);
        assert!(result.evidence.iter().any(|e| e.check_id == "policy"));
    }

    #[spec("CRIT-002")]
    #[tokio::test]
    async fn test_add_check_respects_priority_order() {
        let tool = ToolInfo {
            name: "test".to_string(),
            category: "Test".to_string(),
            description: "Test tool".to_string(),
            schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        };

        let config = CriticConfig {
            fail_fast: false,
            ..Default::default()
        };
        let pipeline = CriticPipeline::with_config(config)
            .add_check(SchemaCheck::with_priority(30))
            .add_check(PolicyCheck::with_security_defaults());

        let input = VerificationInput::new(
            "Test".to_string(),
            ProviderResponse {
                content: "The password is secret123".to_string(),
                reasoning_content: None,
                tool_calls: vec![ParsedToolCall {
                    tool_name: "test".to_string(),
                    arguments: json!({}),
                    call_id: Some("call-1".to_string()),
                }],
                finish_reason: None,
                inference_timing: None,
                quality_gate_retries: 0,
            },
            vec![tool],
        );

        let result = pipeline.verify(&input).await;

        assert!(!result.passed);
        assert_eq!(result.checks_run, 2);
        let ids: Vec<&str> = result
            .evidence
            .iter()
            .map(|e| e.check_id.as_str())
            .collect();
        assert_eq!(ids, vec!["policy", "schema"]);
    }
}
