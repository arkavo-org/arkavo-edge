//! Semantic verification check using ResponseJudge
//!
//! Uses LLM-based validation for semantic coherence.
//! Slower check (~50ms) but catches nuanced issues.

use super::traits::{CheckResult, VerificationCheck, VerificationInput};
use crate::evidence::{CheckSeverity, VerificationEvidence};
use crate::judge::{IssueType, JudgmentResult, ResponseJudge};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// Semantic coherence check using Router's Judge
///
/// Validates:
/// - Response relevance to prompt
/// - Tool usage appropriateness
/// - Refusal detection
/// - Error acknowledgment
pub struct SemanticCheck {
    priority: u32,
    judge: Arc<RwLock<Option<ResponseJudge>>>,
}

impl SemanticCheck {
    /// Create a new semantic check (lazy initialization)
    pub fn new() -> Self {
        Self {
            priority: 100, // Runs last (slowest)
            judge: Arc::new(RwLock::new(None)),
        }
    }

    /// Create with a pre-initialized judge
    pub fn with_judge(judge: ResponseJudge) -> Self {
        Self {
            priority: 100,
            judge: Arc::new(RwLock::new(Some(judge))),
        }
    }

    /// Set priority
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Check if judge is initialized
    ///
    /// Returns error if judge needs to be initialized via `with_judge()`.
    /// This check requires explicit judge initialization since it uses
    /// LLM-based validation which needs a provider.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    async fn ensure_judge(&self) -> crate::Result<()> {
        let is_none = self.judge.read().await.is_none();
        if is_none {
            return Err(crate::Error::Config(
                "Semantic check requires explicit judge initialization via with_judge()"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Map Judge issue type to CheckSeverity
    fn issue_to_severity(issue: &IssueType) -> CheckSeverity {
        match issue {
            IssueType::None => CheckSeverity::Info,
            IssueType::Refusal | IssueType::OffTopic => CheckSeverity::Warning,
            IssueType::MissingToolUse => CheckSeverity::Warning,
            IssueType::HallucinatedTool | IssueType::InvalidParams => CheckSeverity::Error,
            IssueType::ToolErrorIgnored => CheckSeverity::Error,
        }
    }

    /// Convert JudgmentResult to CheckResult
    fn judgment_to_result(&self, judgment: JudgmentResult, latency: u64) -> CheckResult {
        if judgment.passed {
            CheckResult::Pass
        } else {
            let severity = Self::issue_to_severity(&judgment.issue_type);
            let description = judgment
                .reason
                .unwrap_or_else(|| format!("Issue: {}", judgment.issue_type));

            let mut evidence =
                VerificationEvidence::failed(self.id(), &description, severity, latency);

            // Add suggested keywords if present
            if !judgment.suggested_keywords.is_empty() {
                evidence = evidence.with_details(serde_json::json!({
                    "issue_type": judgment.issue_type.to_string(),
                    "suggested_keywords": judgment.suggested_keywords,
                }));
            }

            if severity == CheckSeverity::Warning {
                CheckResult::Warn(evidence)
            } else {
                CheckResult::Fail(evidence)
            }
        }
    }
}

impl Default for SemanticCheck {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VerificationCheck for SemanticCheck {
    fn id(&self) -> &'static str {
        "semantic"
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn verify(&self, input: &VerificationInput) -> CheckResult {
        let start = Instant::now();

        // Check if judge is already initialized
        let needs_init = {
            let guard = self.judge.read().await;
            guard.is_none()
        };

        if needs_init && self.ensure_judge().await.is_err() {
            tracing::debug!(
                check = "semantic",
                "Semantic check skipped (judge not available)"
            );
            return CheckResult::Skip("Judge not available".to_string());
        }

        let guard = self.judge.read().await;
        let judge = match guard.as_ref() {
            Some(j) => j,
            None => {
                return CheckResult::Skip("Judge not initialized".to_string());
            }
        };

        // Run the judge evaluation
        match judge
            .evaluate(&input.prompt, &input.response, &input.available_tools, None)
            .await
        {
            Ok(judgment) => {
                let latency_us = start.elapsed().as_micros() as u64;
                tracing::debug!(
                    check = "semantic",
                    passed = judgment.passed,
                    issue = %judgment.issue_type,
                    latency_us = latency_us,
                    "Semantic check complete"
                );
                self.judgment_to_result(judgment, latency_us)
            }
            Err(e) => {
                let latency_us = start.elapsed().as_micros() as u64;
                tracing::warn!(
                    check = "semantic",
                    error = %e,
                    latency_us = latency_us,
                    "Semantic check failed"
                );
                // Judge error is not a verification failure, just skip
                CheckResult::Skip(format!("Judge error: {e}"))
            }
        }
    }

    fn is_applicable(&self, _input: &VerificationInput) -> bool {
        // Semantic check is always applicable but may skip if judge unavailable
        true
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, clippy::unnecessary_literal_bound)]
mod tests {
    use super::*;
    use arkavo_llm::{Message, Provider, ProviderResponse};
    use arkavo_mcp_tools::ToolInfo;
    use arkavo_test_macros::spec;
    use serde_json::json;

    /// Mock provider for testing
    struct MockProvider {
        response: String,
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn complete(&self, _messages: Vec<Message>) -> arkavo_llm::Result<String> {
            Ok(self.response.clone())
        }

        async fn complete_with_options(
            &self,
            _messages: Vec<Message>,
            _max_tokens: Option<usize>,
        ) -> arkavo_llm::Result<String> {
            Ok(self.response.clone())
        }

        async fn stream(
            &self,
            _messages: Vec<Message>,
        ) -> arkavo_llm::Result<
            Box<
                dyn tokio_stream::Stream<Item = arkavo_llm::Result<arkavo_llm::StreamResponse>>
                    + Send
                    + Unpin,
            >,
        > {
            unimplemented!()
        }

        fn name(&self) -> &str {
            "mock"
        }
    }

    fn create_test_tool() -> ToolInfo {
        ToolInfo {
            name: "search".to_string(),
            category: "Test".to_string(),
            description: "Search for things".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }),
        }
    }

    #[spec("CRIT-005")]
    #[tokio::test]
    async fn test_semantic_with_mock_pass() {
        let mock_provider = Arc::new(MockProvider {
            response: "VERDICT: PASS\nISSUE: none".to_string(),
        });

        let judge = ResponseJudge::new(mock_provider);
        let check = SemanticCheck::with_judge(judge);

        let response = ProviderResponse {
            content: "Here are the search results.".to_string(),
            reasoning_content: None,
            tool_calls: vec![arkavo_llm::tool_parser::ParsedToolCall {
                tool_name: "search".to_string(),
                arguments: json!({"query": "test"}),
                call_id: None,
            }],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
            ..Default::default()
        };

        let input = VerificationInput::new(
            "Search for test".to_string(),
            response,
            vec![create_test_tool()],
        );

        let result = check.verify(&input).await;
        assert!(result.is_pass());
    }

    #[spec("CRIT-005")]
    #[tokio::test]
    async fn test_semantic_with_mock_fail() {
        let mock_provider = Arc::new(MockProvider {
            response: "VERDICT: FAIL\nREASON: Tool does not exist\nISSUE: hallucinated_tool"
                .to_string(),
        });

        let judge = ResponseJudge::new(mock_provider);
        let check = SemanticCheck::with_judge(judge);

        let response = ProviderResponse {
            content: "Using nonexistent tool".to_string(),
            reasoning_content: None,
            tool_calls: vec![arkavo_llm::tool_parser::ParsedToolCall {
                tool_name: "fake_tool".to_string(),
                arguments: json!({}),
                call_id: None,
            }],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
            ..Default::default()
        };

        let input = VerificationInput::new(
            "Search for test".to_string(),
            response,
            vec![create_test_tool()],
        );

        let result = check.verify(&input).await;
        // Note: The heuristic check in Judge catches this first
        assert!(result.is_fail());
    }

    #[tokio::test]
    async fn test_issue_to_severity() {
        assert_eq!(
            SemanticCheck::issue_to_severity(&IssueType::None),
            CheckSeverity::Info
        );
        assert_eq!(
            SemanticCheck::issue_to_severity(&IssueType::Refusal),
            CheckSeverity::Warning
        );
        assert_eq!(
            SemanticCheck::issue_to_severity(&IssueType::HallucinatedTool),
            CheckSeverity::Error
        );
    }
}
