use async_trait::async_trait;
use serde_json::Value;

// Re-export from arkavo-mcp for consistency
pub use arkavo_mcp::ToolSchema;

/// Core trait for MCP tools
#[async_trait]
pub trait Tool: Send + Sync {
    /// Execute the tool with the given parameters
    async fn execute(&self, params: Value) -> crate::Result<Value>;

    /// Get the tool's schema definition
    fn schema(&self) -> &ToolSchema;
}

// Helper function to create a standard error response
pub fn error_response(message: &str) -> Value {
    serde_json::json!({
        "success": false,
        "error": message
    })
}

// Helper function to create a standard success response
pub fn success_response(data: Value) -> Value {
    serde_json::json!({
        "success": true,
        "data": data
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    /// Validate that `params` contains all keys listed in the schema's `required` array.
    ///
    /// Test-only helper that lets the `TestCalculatorTool` enforce its JSON Schema contract
    /// without pulling in a full JSON Schema validator.
    fn validate_required_params(schema: &ToolSchema, params: &Value) -> crate::Result<()> {
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        for req in &required {
            let Some(name) = req.as_str() else {
                continue;
            };
            if params.get(name).is_none() {
                return Err(crate::ToolError::InvalidParams(format!(
                    "Missing required parameter: {name}"
                )));
            }
        }
        Ok(())
    }

    /// A minimal in-memory tool used to exercise the `Tool` trait contract under test.
    struct TestCalculatorTool {
        schema: ToolSchema,
    }

    impl TestCalculatorTool {
        fn new() -> Self {
            Self {
                schema: ToolSchema {
                    name: "test_calculator".to_string(),
                    aliases: None,
                    description: "Simple arithmetic tool for testing the Tool trait".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "a": { "type": "number" },
                            "b": { "type": "number" },
                            "operation": {
                                "type": "string",
                                "enum": ["add", "subtract", "multiply"]
                            }
                        },
                        "required": ["a", "b"]
                    }),
                },
            }
        }
    }

    #[async_trait]
    impl Tool for TestCalculatorTool {
        async fn execute(&self, params: Value) -> crate::Result<Value> {
            validate_required_params(&self.schema, &params)?;

            let a = params["a"].as_f64().ok_or_else(|| {
                crate::ToolError::InvalidParams("'a' must be a number".to_string())
            })?;
            let b = params["b"].as_f64().ok_or_else(|| {
                crate::ToolError::InvalidParams("'b' must be a number".to_string())
            })?;

            let operation = params["operation"].as_str().unwrap_or("add");
            let result = match operation {
                "add" => a + b,
                "subtract" => a - b,
                "multiply" => a * b,
                _ => {
                    return Ok(error_response(&format!(
                        "Unsupported operation: {operation}"
                    )));
                }
            };

            Ok(success_response(json!({
                "operation": operation,
                "result": result
            })))
        }

        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
    }

    #[spec("MCP-003")]
    #[tokio::test]
    async fn test_execute_with_valid_params() {
        let tool = TestCalculatorTool::new();
        let params = json!({ "a": 5, "b": 3, "operation": "multiply" });

        let result = tool.execute(params).await.expect("execute should succeed");

        assert_eq!(result["success"], true);
        assert_eq!(result["data"]["operation"], "multiply");
        assert_eq!(result["data"]["result"], 15.0);
    }

    #[spec("MCP-003")]
    #[tokio::test]
    async fn test_execute_missing_required_param_returns_error() {
        let tool = TestCalculatorTool::new();
        let params = json!({ "a": 5 });

        let err = tool
            .execute(params)
            .await
            .expect_err("missing required param should fail");

        assert!(
            err.to_string().contains("Missing required parameter: b"),
            "error should name the missing parameter: {err}"
        );
    }

    #[spec("MCP-003")]
    #[tokio::test]
    async fn test_execute_invalid_param_type_returns_error() {
        let tool = TestCalculatorTool::new();
        let params = json!({ "a": "not-a-number", "b": 3 });

        let err = tool
            .execute(params)
            .await
            .expect_err("invalid param type should fail");

        assert!(
            err.to_string().contains("'a' must be a number"),
            "error should describe the invalid parameter: {err}"
        );
    }

    #[spec("MCP-003")]
    #[tokio::test]
    async fn test_execute_captures_runtime_error_in_json() {
        let tool = TestCalculatorTool::new();
        let params = json!({ "a": 5, "b": 3, "operation": "divide" });

        let result = tool
            .execute(params)
            .await
            .expect("execute should return a Value");

        assert_eq!(result["success"], false);
        assert!(
            result["error"]
                .as_str()
                .unwrap()
                .contains("Unsupported operation"),
            "error JSON should describe the unsupported operation"
        );
    }

    #[spec("MCP-003")]
    #[tokio::test]
    async fn test_execute_default_optional_param() {
        let tool = TestCalculatorTool::new();
        let params = json!({ "a": 10, "b": 4 });

        let result = tool.execute(params).await.expect("execute should succeed");

        assert_eq!(result["success"], true);
        assert_eq!(result["data"]["operation"], "add");
        assert_eq!(result["data"]["result"], 14.0);
    }

    #[test]
    fn test_error_response_format() {
        let response = error_response("something went wrong");
        assert_eq!(response["success"], false);
        assert_eq!(response["error"], "something went wrong");
    }

    #[test]
    fn test_success_response_format() {
        let response = success_response(json!({ "value": 42 }));
        assert_eq!(response["success"], true);
        assert_eq!(response["data"]["value"], 42);
    }

    #[test]
    fn test_validate_required_params_accepts_complete() {
        let schema = ToolSchema {
            name: "test".to_string(),
            aliases: None,
            description: "test".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["x", "y"]
            }),
        };
        let params = json!({ "x": 1, "y": 2, "z": 3 });
        assert!(validate_required_params(&schema, &params).is_ok());
    }

    #[test]
    fn test_validate_required_params_rejects_incomplete() {
        let schema = ToolSchema {
            name: "test".to_string(),
            aliases: None,
            description: "test".to_string(),
            parameters: json!({
                "type": "object",
                "required": ["x", "y"]
            }),
        };
        let params = json!({ "x": 1 });
        let err = validate_required_params(&schema, &params).unwrap_err();
        assert!(err.to_string().contains("Missing required parameter: y"));
    }

    /// Compile-time guard that the `Tool` trait bounds are actually useful for multi-threaded
    /// async runtimes and that concrete tool implementations can be held in shared registries.
    fn assert_tool_is_send_sync<T: Tool>() {}

    #[test]
    fn test_tool_is_send_sync() {
        assert_tool_is_send_sync::<TestCalculatorTool>();
    }
}
