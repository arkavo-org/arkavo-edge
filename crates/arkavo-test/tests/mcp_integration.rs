#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use arkavo_test::mcp::server::ToolRequest;
    use arkavo_test::{TestError, TestHarness};
    use serde_json::json;

    #[tokio::test]
    async fn test_mcp_server_query_state() -> Result<(), TestError> {
        let harness = TestHarness::new()?;
        let mcp_server = harness.mcp_server();

        let request = ToolRequest {
            tool_name: "query_state".to_string(),
            params: json!({
                "entity": "app"
            }),
        };

        let response = mcp_server.call_tool(request).await?;
        let result = response.result;

        assert!(result.get("state").is_some());
        assert!(result.get("timestamp").is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_mcp_server_execute_action() -> Result<(), TestError> {
        let harness = TestHarness::new()?;
        let mcp_server = harness.mcp_server();

        let request = ToolRequest {
            tool_name: "ui_interaction".to_string(),
            params: json!({
                "action": "tap",
                "target": {
                    "x": 100,
                    "y": 100
                }
            }),
        };

        let response = mcp_server.call_tool(request).await?;
        let result = response.result;

        // The tap might return an error if no device is active, or success if it works
        if let Some(error) = result.get("error") {
            // It's ok if we get NO_ACTIVE_DEVICE error
            assert_eq!(
                error.get("code").and_then(|v| v.as_str()),
                Some("NO_ACTIVE_DEVICE")
            );
        } else {
            // Otherwise we should have success
            assert_eq!(result.get("success").and_then(|v| v.as_bool()), Some(true));
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_mcp_server_device_info() -> Result<(), TestError> {
        let harness = TestHarness::new()?;
        let mcp_server = harness.mcp_server();

        let request = ToolRequest {
            tool_name: "device_management".to_string(),
            params: json!({
                "action": "get_active"
            }),
        };

        let response = mcp_server.call_tool(request).await?;
        let result = response.result;

        // The response should contain device field (could be null)
        assert!(result.get("device").is_some());
        assert!(result.get("active").is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_mcp_server_update_state() -> Result<(), TestError> {
        let harness = TestHarness::new()?;
        let mcp_server = harness.mcp_server();

        // Update state using mutate_state
        let update_request = ToolRequest {
            tool_name: "mutate_state".to_string(),
            params: json!({
                "entity": "test_flow",
                "action": "set",
                "data": {
                    "current_step": 1,
                    "status": "in_progress"
                }
            }),
        };

        let response = mcp_server.call_tool(update_request).await?;
        assert_eq!(
            response.result.get("success").and_then(|v| v.as_bool()),
            Some(true)
        );

        // Query the state back
        let query_request = ToolRequest {
            tool_name: "query_state".to_string(),
            params: json!({
                "entity": "test_flow"
            }),
        };

        let response = mcp_server.call_tool(query_request).await?;
        let result = response.result;

        let state = result.get("state").and_then(|s| s.get("test_flow"));
        assert_eq!(
            state
                .and_then(|s| s.get("current_step"))
                .and_then(|v| v.as_i64()),
            Some(1)
        );
        assert_eq!(
            state.and_then(|s| s.get("status")).and_then(|v| v.as_str()),
            Some("in_progress")
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_mcp_server_invalid_tool() -> Result<(), TestError> {
        let harness = TestHarness::new()?;
        let mcp_server = harness.mcp_server();

        let request = ToolRequest {
            tool_name: "non_existent_tool".to_string(),
            params: json!({}),
        };

        let result = mcp_server.call_tool(request).await;
        assert!(result.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_mcp_server_missing_params() -> Result<(), TestError> {
        let harness = TestHarness::new()?;
        let mcp_server = harness.mcp_server();

        // Query state without entity
        let request = ToolRequest {
            tool_name: "query_state".to_string(),
            params: json!({}),
        };

        let result = mcp_server.call_tool(request).await;
        assert!(result.is_err());

        Ok(())
    }
}
