//! Core traits for verification checks

use crate::evidence::VerificationEvidence;
use arkavo_llm::ProviderResponse;
use arkavo_mcp_tools::ToolInfo;
use async_trait::async_trait;

/// Input for verification checks
#[derive(Debug, Clone)]
pub struct VerificationInput {
    /// The original prompt/task description
    pub prompt: String,
    /// The LLM response to verify
    pub response: ProviderResponse,
    /// Available tools
    pub available_tools: Vec<ToolInfo>,
    /// Task category (for category-specific checks)
    pub task_category: Option<String>,
    /// Additional context
    pub context: Option<serde_json::Value>,
}

impl VerificationInput {
    /// Create a new verification input
    pub fn new(prompt: String, response: ProviderResponse, tools: Vec<ToolInfo>) -> Self {
        Self {
            prompt,
            response,
            available_tools: tools,
            task_category: None,
            context: None,
        }
    }

    /// Set the task category
    pub fn with_category(mut self, category: &str) -> Self {
        self.task_category = Some(category.to_string());
        self
    }

    /// Set additional context
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = Some(context);
        self
    }
}

/// Result of a verification check
#[derive(Debug, Clone)]
pub enum CheckResult {
    /// Check passed
    Pass,
    /// Check failed with evidence
    Fail(VerificationEvidence),
    /// Check produced a warning but didn't fail
    Warn(VerificationEvidence),
    /// Check was skipped (not applicable)
    Skip(String),
    /// Check requires human approval (HITL)
    NeedsApproval(VerificationEvidence),
}

impl CheckResult {
    /// Check if the result is a pass
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// Check if the result is a failure
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail(_))
    }

    /// Check if the result needs human approval
    pub fn needs_approval(&self) -> bool {
        matches!(self, Self::NeedsApproval(_))
    }

    /// Get the evidence if present
    pub fn evidence(&self) -> Option<&VerificationEvidence> {
        match self {
            Self::Fail(e) | Self::Warn(e) | Self::NeedsApproval(e) => Some(e),
            _ => None,
        }
    }
}

/// A verification check that can be run as part of the pipeline
#[async_trait]
pub trait VerificationCheck: Send + Sync {
    /// Unique identifier for this check
    fn id(&self) -> &'static str;

    /// Priority (lower = runs first)
    fn priority(&self) -> u32;

    /// Run the verification check
    async fn verify(&self, input: &VerificationInput) -> CheckResult;

    /// Whether this check is applicable to the given input
    fn is_applicable(&self, _input: &VerificationInput) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::ProviderResponse;

    #[test]
    fn test_verification_input_creation() {
        let response = ProviderResponse {
            content: "Test".to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
        };

        let input = VerificationInput::new("prompt".to_string(), response, vec![])
            .with_category("code")
            .with_context(serde_json::json!({"key": "value"}));

        assert_eq!(input.prompt, "prompt");
        assert_eq!(input.task_category, Some("code".to_string()));
        assert!(input.context.is_some());
    }

    #[test]
    fn test_check_result_methods() {
        assert!(CheckResult::Pass.is_pass());
        assert!(!CheckResult::Pass.is_fail());
        assert!(!CheckResult::Pass.needs_approval());

        let evidence =
            VerificationEvidence::failed("test", "reason", crate::CheckSeverity::Error, 1);
        let fail = CheckResult::Fail(evidence);
        assert!(fail.is_fail());
        assert!(!fail.is_pass());
        assert!(fail.evidence().is_some());
    }
}
