//! Schema validation check
//!
//! Validates tool calls against their schemas using ResponseValidator.
//! This is the fastest check (<1ms) and should run first.

use super::traits::{CheckResult, VerificationCheck, VerificationInput};
use crate::evidence::{CheckSeverity, VerificationEvidence};
use crate::validator::ResponseValidator;
use async_trait::async_trait;
use std::time::Instant;

/// Schema validation check
///
/// Validates that:
/// - Tool calls reference existing tools
/// - Required parameters are provided
/// - Parameter types match schema
pub struct SchemaCheck {
    priority: u32,
}

impl SchemaCheck {
    /// Create a new schema check
    pub fn new() -> Self {
        Self { priority: 10 } // Runs first
    }

    /// Create with custom priority
    pub fn with_priority(priority: u32) -> Self {
        Self { priority }
    }
}

impl Default for SchemaCheck {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VerificationCheck for SchemaCheck {
    fn id(&self) -> &'static str {
        "schema"
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    async fn verify(&self, input: &VerificationInput) -> CheckResult {
        let start = Instant::now();

        // Skip if no tool calls
        if input.response.tool_calls.is_empty() {
            return CheckResult::Skip("No tool calls to validate".to_string());
        }

        // Use router's validator
        let validator = ResponseValidator::new(&input.available_tools);

        match validator.quick_validate(&input.response) {
            Ok(()) => {
                let latency_us = start.elapsed().as_micros() as u64;
                tracing::debug!(
                    check = "schema",
                    latency_us = latency_us,
                    "Schema check passed"
                );
                CheckResult::Pass
            }
            Err(validation_error) => {
                let latency_us = start.elapsed().as_micros() as u64;
                let evidence = VerificationEvidence::failed(
                    self.id(),
                    &validation_error.to_string(),
                    CheckSeverity::Error,
                    latency_us,
                );
                tracing::warn!(
                    check = "schema",
                    error = %validation_error,
                    latency_us = latency_us,
                    "Schema check failed"
                );
                CheckResult::Fail(evidence)
            }
        }
    }

    fn is_applicable(&self, input: &VerificationInput) -> bool {
        // Only applicable if there are tool calls
        !input.response.tool_calls.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, clippy::unnecessary_literal_bound)]
mod tests {
    use super::*;
    use arkavo_llm::ProviderResponse;
    use arkavo_llm::tool_parser::ParsedToolCall;
    use arkavo_mcp_tools::ToolInfo;
    use arkavo_test_macros::spec;
    use serde_json::json;

    fn create_test_tool(name: &str) -> ToolInfo {
        ToolInfo {
            name: name.to_string(),
            category: "Test".to_string(),
            description: "Test tool".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        }
    }

    #[spec("CRIT-004")]
    #[tokio::test]
    async fn test_schema_check_pass() {
        let check = SchemaCheck::new();
        let tools = vec![create_test_tool("search")];

        let response = ProviderResponse {
            response_items: Vec::new(),
            content: "Searching...".to_string(),
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "search".to_string(),
                arguments: json!({"query": "test"}),
                call_id: None,
            }],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        };

        let input = VerificationInput::new("Find test".to_string(), response, tools);
        let result = check.verify(&input).await;

        assert!(result.is_pass());
    }

    #[spec("CRIT-004")]
    #[tokio::test]
    async fn test_schema_check_hallucinated_tool() {
        let check = SchemaCheck::new();
        let tools = vec![create_test_tool("search")];

        let response = ProviderResponse {
            response_items: Vec::new(),
            content: "Using tool...".to_string(),
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "nonexistent".to_string(),
                arguments: json!({}),
                call_id: None,
            }],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        };

        let input = VerificationInput::new("Test".to_string(), response, tools);
        let result = check.verify(&input).await;

        assert!(result.is_fail());
        let evidence = result.evidence().unwrap();
        assert!(evidence.description.contains("nonexistent"));
    }

    #[spec("CRIT-004")]
    #[tokio::test]
    async fn test_schema_check_missing_param() {
        let check = SchemaCheck::new();
        let tools = vec![create_test_tool("search")];

        let response = ProviderResponse {
            response_items: Vec::new(),
            content: "Searching...".to_string(),
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "search".to_string(),
                arguments: json!({}), // Missing required 'query'
                call_id: None,
            }],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        };

        let input = VerificationInput::new("Find test".to_string(), response, tools);
        let result = check.verify(&input).await;

        assert!(result.is_fail());
        let evidence = result.evidence().unwrap();
        assert!(evidence.description.contains("query"));
    }

    #[tokio::test]
    async fn test_schema_check_skip_no_tools() {
        let check = SchemaCheck::new();

        let response = ProviderResponse {
            response_items: Vec::new(),
            content: "No tools needed".to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        };

        let input = VerificationInput::new("Test".to_string(), response, vec![]);
        let result = check.verify(&input).await;

        assert!(matches!(result, CheckResult::Skip(_)));
    }
}
