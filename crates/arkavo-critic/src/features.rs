//! Boolean feature extraction for TØR-G circuit inputs.
//!
//! This module maps `VerificationInput` data to boolean values
//! for evaluation by TØR-G circuits.

use arkavo_torg_circuits::CircuitFeature;

use crate::checks::VerificationInput;

/// Boolean feature IDs for circuit inputs.
///
/// Each variant extracts a boolean value from `VerificationInput`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FeatureId {
    // Tool intent
    /// True if response contains any tool calls
    HasToolCalls,
    /// True if any tool name contains "read"
    IntentIsRead,
    /// True if any tool name contains "write"
    IntentIsWrite,
    /// True if any tool name contains "delete"
    IntentIsDelete,
    /// True if any tool name contains "execute"
    IntentIsExecute,

    // Response properties
    /// True if response content is empty
    ResponseIsEmpty,
    /// True if response contains code blocks
    ResponseHasCode,

    // Category
    /// True if task_category is "security"
    CategoryIsSecurity,
    /// True if task_category is "database"
    CategoryIsDatabase,

    // Custom (for user-defined policies)
    /// Custom boolean field from context JSON
    Custom(String),
}

impl FeatureId {
    fn check_tool_intent(input: &VerificationInput, intent: &str) -> bool {
        input
            .response
            .tool_calls
            .iter()
            .any(|tc| tc.tool_name.to_lowercase().contains(intent))
    }

    fn extract_custom(input: &VerificationInput, key: &str) -> bool {
        input
            .context
            .as_ref()
            .and_then(|ctx| ctx.get(key))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }
}

impl CircuitFeature for FeatureId {
    type Input = VerificationInput;

    fn extract(&self, input: &Self::Input) -> bool {
        match self {
            Self::HasToolCalls => !input.response.tool_calls.is_empty(),
            Self::IntentIsRead => Self::check_tool_intent(input, "read"),
            Self::IntentIsWrite => Self::check_tool_intent(input, "write"),
            Self::IntentIsDelete => Self::check_tool_intent(input, "delete"),
            Self::IntentIsExecute => Self::check_tool_intent(input, "execute"),
            Self::ResponseIsEmpty => input.response.content.is_empty(),
            Self::ResponseHasCode => input.response.content.contains("```"),
            Self::CategoryIsSecurity => input.task_category.as_deref() == Some("security"),
            Self::CategoryIsDatabase => input.task_category.as_deref() == Some("database"),
            Self::Custom(key) => Self::extract_custom(input, key),
        }
    }

    fn name(&self) -> String {
        match self {
            Self::HasToolCalls => "HasToolCalls".into(),
            Self::IntentIsRead => "IntentIsRead".into(),
            Self::IntentIsWrite => "IntentIsWrite".into(),
            Self::IntentIsDelete => "IntentIsDelete".into(),
            Self::IntentIsExecute => "IntentIsExecute".into(),
            Self::ResponseIsEmpty => "ResponseIsEmpty".into(),
            Self::ResponseHasCode => "ResponseHasCode".into(),
            Self::CategoryIsSecurity => "CategoryIsSecurity".into(),
            Self::CategoryIsDatabase => "CategoryIsDatabase".into(),
            Self::Custom(key) => format!("Custom({key})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::{ParsedToolCall, ProviderResponse};

    fn make_response(content: &str, tool_calls: Vec<ParsedToolCall>) -> ProviderResponse {
        ProviderResponse {
            response_items: Vec::new(),
            content: content.to_string(),
            reasoning_content: None,
            tool_calls,
            finish_reason: None,
            inference_timing: None,
            quality_gate_retries: 0,
        }
    }

    #[test]
    fn test_has_tool_calls() {
        let input = VerificationInput::new("test".into(), make_response("", vec![]), vec![]);
        assert!(!FeatureId::HasToolCalls.extract(&input));

        let input_with_tools = VerificationInput::new(
            "test".into(),
            make_response(
                "",
                vec![ParsedToolCall {
                    tool_name: "file_read".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        );
        assert!(FeatureId::HasToolCalls.extract(&input_with_tools));
    }

    #[test]
    fn test_intent_detection() {
        let input = VerificationInput::new(
            "test".into(),
            make_response(
                "",
                vec![ParsedToolCall {
                    tool_name: "file_write".into(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
            ),
            vec![],
        );

        assert!(FeatureId::IntentIsWrite.extract(&input));
        assert!(!FeatureId::IntentIsRead.extract(&input));
        assert!(!FeatureId::IntentIsDelete.extract(&input));
    }

    #[test]
    fn test_response_has_code() {
        let input = VerificationInput::new(
            "test".into(),
            make_response("```rust\nfn main() {}\n```", vec![]),
            vec![],
        );
        assert!(FeatureId::ResponseHasCode.extract(&input));

        let input_no_code =
            VerificationInput::new("test".into(), make_response("Hello world", vec![]), vec![]);
        assert!(!FeatureId::ResponseHasCode.extract(&input_no_code));
    }

    #[test]
    fn test_category() {
        let input = VerificationInput::new("test".into(), make_response("", vec![]), vec![])
            .with_category("security");

        assert!(FeatureId::CategoryIsSecurity.extract(&input));
        assert!(!FeatureId::CategoryIsDatabase.extract(&input));
    }

    #[test]
    fn test_custom_feature() {
        let input = VerificationInput::new("test".into(), make_response("", vec![]), vec![])
            .with_context(serde_json::json!({"is_admin": true}));

        assert!(FeatureId::Custom("is_admin".into()).extract(&input));
        assert!(!FeatureId::Custom("is_guest".into()).extract(&input));
    }
}
