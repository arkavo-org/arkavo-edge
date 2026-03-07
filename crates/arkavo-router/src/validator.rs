use arkavo_llm::ProviderResponse;
use arkavo_llm::tool_parser::ParsedToolCall;
use arkavo_mcp_tools::ToolInfo;
use serde_json::Value;
use std::collections::HashSet;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    HallucinatedTool {
        tool_name: String,
    },
    MissingRequiredParam {
        tool_name: String,
        param: String,
    },
    InvalidParamType {
        tool_name: String,
        param: String,
        expected: String,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::HallucinatedTool { tool_name } => {
                write!(f, "Tool does not exist: {tool_name}")
            }
            ValidationError::MissingRequiredParam { tool_name, param } => {
                write!(
                    f,
                    "Missing required parameter '{param}' for tool '{tool_name}'"
                )
            }
            ValidationError::InvalidParamType {
                tool_name,
                param,
                expected,
            } => write!(
                f,
                "Invalid type for parameter '{param}' in tool '{tool_name}' (expected {expected})"
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

impl ValidationError {
    /// Generate a specific, actionable fix suggestion for the model to retry with.
    pub fn fix_suggestion(&self, available_tools: &[&str]) -> String {
        match self {
            ValidationError::HallucinatedTool { tool_name } => {
                let tools_list = if available_tools.len() <= 10 {
                    available_tools.join(", ")
                } else {
                    let mut list = available_tools[..10].join(", ");
                    let _ = write!(list, " (and {} more)", available_tools.len() - 10);
                    list
                };
                format!("'{tool_name}' is not a valid tool. Available tools: {tools_list}")
            }
            ValidationError::MissingRequiredParam { tool_name, param } => {
                format!(
                    "Add the required '{param}' parameter to your '{tool_name}' call. Example:\n```{tool_name}\n{param}: <value>\n```"
                )
            }
            ValidationError::InvalidParamType {
                tool_name,
                param,
                expected,
            } => {
                let example = match expected.as_str() {
                    "string" => "\"example text\"",
                    "number" | "integer" => "42",
                    "boolean" => "true",
                    "array" => "[1, 2, 3]",
                    "object" => "{\"key\": \"value\"}",
                    _ => "<value>",
                };
                format!(
                    "Parameter '{param}' in '{tool_name}' must be type {expected}. Example: {param}: {example}"
                )
            }
        }
    }
}

pub struct ResponseValidator {
    available_tools: HashSet<String>,
    tool_schemas: Vec<ToolInfo>,
}

impl ResponseValidator {
    pub fn new(available_tools: &[ToolInfo]) -> Self {
        let tool_names = available_tools
            .iter()
            .map(|t| t.name.clone())
            .collect::<HashSet<_>>();

        Self {
            available_tools: tool_names,
            tool_schemas: available_tools.to_vec(),
        }
    }

    pub fn quick_validate(&self, response: &ProviderResponse) -> Result<(), ValidationError> {
        if response.tool_calls.is_empty() {
            return Ok(());
        }

        for call in &response.tool_calls {
            if !self.available_tools.contains(&call.tool_name) {
                return Err(ValidationError::HallucinatedTool {
                    tool_name: call.tool_name.clone(),
                });
            }

            if let Some(tool_info) = self.get_tool_schema(&call.tool_name) {
                self.validate_tool_params(call, &tool_info.schema)?;
            }
        }

        Ok(())
    }

    fn get_tool_schema(&self, tool_name: &str) -> Option<&ToolInfo> {
        self.tool_schemas.iter().find(|t| t.name == tool_name)
    }

    fn validate_tool_params(
        &self,
        call: &ParsedToolCall,
        schema: &Value,
    ) -> Result<(), ValidationError> {
        let properties = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .cloned()
            .unwrap_or_default();

        let required_params = schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();

        let provided_args = call
            .arguments
            .as_object()
            .map(|obj| obj.keys().map(|k| k.as_str()).collect::<HashSet<_>>())
            .unwrap_or_default();

        for required_param in &required_params {
            if !provided_args.contains(required_param) {
                return Err(ValidationError::MissingRequiredParam {
                    tool_name: call.tool_name.clone(),
                    param: (*required_param).to_string(),
                });
            }
        }

        if let Some(args_obj) = call.arguments.as_object() {
            for (param_name, param_value) in args_obj {
                if let Some(param_schema) = properties.get(param_name)
                    && let Some(expected_type) = param_schema.get("type").and_then(|t| t.as_str())
                    && !self.check_type_match(param_value, expected_type)
                {
                    return Err(ValidationError::InvalidParamType {
                        tool_name: call.tool_name.clone(),
                        param: param_name.clone(),
                        expected: expected_type.to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    fn check_type_match(&self, value: &Value, expected_type: &str) -> bool {
        match expected_type {
            "string" => value.is_string(),
            "number" | "integer" => value.is_number(),
            "boolean" => value.is_boolean(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_tool_info(name: &str, required_params: &[&str]) -> ToolInfo {
        ToolInfo {
            name: name.to_string(),
            category: "Test".to_string(),
            description: "Test tool".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "number" },
                },
                "required": required_params,
            }),
        }
    }

    #[test]
    fn test_valid_response() {
        let tools = vec![create_test_tool_info("search", &["query"])];
        let validator = ResponseValidator::new(&tools);

        let response = ProviderResponse {
            content: "Searching...".to_string(),
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "search".to_string(),
                arguments: json!({"query": "test"}),
                call_id: None,
            }],
            finish_reason: None,
        };

        assert!(validator.quick_validate(&response).is_ok());
    }

    #[test]
    fn test_hallucinated_tool() {
        let tools = vec![create_test_tool_info("search", &["query"])];
        let validator = ResponseValidator::new(&tools);

        let response = ProviderResponse {
            content: "Using tool...".to_string(),
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "nonexistent_tool".to_string(),
                arguments: json!({}),
                call_id: None,
            }],
            finish_reason: None,
        };

        let result = validator.quick_validate(&response);
        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::HallucinatedTool { tool_name } => {
                assert_eq!(tool_name, "nonexistent_tool");
            }
            _ => panic!("Expected HallucinatedTool error"),
        }
    }

    #[test]
    fn test_missing_required_param() {
        let tools = vec![create_test_tool_info("search", &["query"])];
        let validator = ResponseValidator::new(&tools);

        let response = ProviderResponse {
            content: "Searching...".to_string(),
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "search".to_string(),
                arguments: json!({"limit": 10}),
                call_id: None,
            }],
            finish_reason: None,
        };

        let result = validator.quick_validate(&response);
        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::MissingRequiredParam { tool_name, param } => {
                assert_eq!(tool_name, "search");
                assert_eq!(param, "query");
            }
            _ => panic!("Expected MissingRequiredParam error"),
        }
    }

    #[test]
    fn test_invalid_param_type() {
        let tools = vec![create_test_tool_info("search", &["query"])];
        let validator = ResponseValidator::new(&tools);

        let response = ProviderResponse {
            content: "Searching...".to_string(),
            reasoning_content: None,
            tool_calls: vec![ParsedToolCall {
                tool_name: "search".to_string(),
                arguments: json!({"query": 123}),
                call_id: None,
            }],
            finish_reason: None,
        };

        let result = validator.quick_validate(&response);
        assert!(result.is_err());
        match result.unwrap_err() {
            ValidationError::InvalidParamType {
                tool_name,
                param,
                expected,
            } => {
                assert_eq!(tool_name, "search");
                assert_eq!(param, "query");
                assert_eq!(expected, "string");
            }
            _ => panic!("Expected InvalidParamType error"),
        }
    }

    #[test]
    fn test_fix_suggestion_hallucinated_tool() {
        let err = ValidationError::HallucinatedTool {
            tool_name: "fake_tool".to_string(),
        };
        let available = vec!["search", "read_file", "write_file"];
        let fix = err.fix_suggestion(&available);
        assert!(fix.contains("fake_tool"));
        assert!(fix.contains("search"));
        assert!(fix.contains("read_file"));
    }

    #[test]
    fn test_fix_suggestion_missing_param() {
        let err = ValidationError::MissingRequiredParam {
            tool_name: "search".to_string(),
            param: "query".to_string(),
        };
        let fix = err.fix_suggestion(&[]);
        assert!(fix.contains("query"));
        assert!(fix.contains("search"));
        assert!(fix.contains("```search"));
    }

    #[test]
    fn test_fix_suggestion_invalid_type() {
        let err = ValidationError::InvalidParamType {
            tool_name: "search".to_string(),
            param: "limit".to_string(),
            expected: "number".to_string(),
        };
        let fix = err.fix_suggestion(&[]);
        assert!(fix.contains("limit"));
        assert!(fix.contains("number"));
        assert!(fix.contains("42"));
    }

    #[test]
    fn test_no_tool_calls() {
        let tools = vec![create_test_tool_info("search", &["query"])];
        let validator = ResponseValidator::new(&tools);

        let response = ProviderResponse {
            content: "No tools needed".to_string(),
            reasoning_content: None,
            tool_calls: vec![],
            finish_reason: None,
        };

        assert!(validator.quick_validate(&response).is_ok());
    }
}
