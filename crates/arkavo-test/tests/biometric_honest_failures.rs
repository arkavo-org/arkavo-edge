#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use arkavo_test::mcp::server::McpTestServer;
    use serde_json::json;

    #[tokio::test]
    async fn test_biometric_tools_return_failures_when_automation_fails() {
        // Initialize MCP server
        let server = McpTestServer::new().expect("Failed to create MCP server");

        // First, add a test device to avoid device not found errors
        let device_request = arkavo_test::mcp::server::ToolRequest {
            tool_name: "device_management".to_string(),
            params: json!({
                "action": "add_test_device",
                "device_id": "test-device-123",
                "name": "Test iPhone",
                "device_type": "iPhone 15",
                "runtime": "iOS 17.0"
            }),
        };

        // Add the test device (this may fail if not supported, which is OK)
        let _ = server.call_tool(device_request).await;

        // Test biometric_auth tool returns failure (not fake success)
        let request = arkavo_test::mcp::server::ToolRequest {
            tool_name: "biometric_auth".to_string(),
            params: json!({
                "action": "match",
                "biometric_type": "face_id",
                "device_id": "test-device-123"
            }),
        };

        let response = server.call_tool(request).await.expect("Tool call failed");
        let result = response.result;

        // Debug print the result
        eprintln!(
            "biometric_auth result: {}",
            serde_json::to_string_pretty(&result).unwrap()
        );

        // The tool should either:
        // 1. Return success:false with error details (preferred)
        // 2. Return an error object (also acceptable)
        // But it should NOT return success:true when it can't actually perform the action

        let has_success_false = result.get("success").and_then(|v| v.as_bool()) == Some(false);
        let has_error = result.get("error").is_some();

        assert!(
            has_success_false || has_error,
            "biometric_auth should indicate failure either via success:false or error object, not fake success. Got: {:?}",
            result
        );

        // If it has a success field, it should be false
        if let Some(success) = result.get("success") {
            assert_eq!(
                success.as_bool(),
                Some(false),
                "When success field is present, it must be false for failed operations"
            );
        }
    }

    #[tokio::test]
    async fn test_biometric_enrollment_returns_proper_error() {
        let server = McpTestServer::new().expect("Failed to create MCP server");

        // Add a test device first
        let _ = server
            .call_tool(arkavo_test::mcp::server::ToolRequest {
                tool_name: "device_management".to_string(),
                params: json!({
                    "action": "set_active",
                    "device_id": "test-device-456"
                }),
            })
            .await;

        let request = arkavo_test::mcp::server::ToolRequest {
            tool_name: "biometric_auth".to_string(),
            params: json!({
                "action": "enroll",
                "biometric_type": "face_id",
                "device_id": "test-device-456"
            }),
        };

        let response = server.call_tool(request).await.expect("Tool call failed");
        let result = response.result;

        eprintln!(
            "enrollment result: {}",
            serde_json::to_string_pretty(&result).unwrap()
        );

        // Check if it's an error response or a failed attempt
        let has_error_object = result.get("error").is_some();
        let has_success_false = result.get("success").and_then(|v| v.as_bool()) == Some(false);

        if has_error_object {
            // Device error is OK - we're testing the response format
            let error = result.get("error").unwrap();
            assert!(
                error.get("code").is_some() || error.get("message").is_some(),
                "Error should have code or message"
            );
        } else if has_success_false {
            // Proper failure response
            let error = result.get("error").unwrap();
            assert!(error.get("code").is_some(), "Error should have a code");

            let error_obj = error.as_object().unwrap();
            if let Some(details) = error_obj.get("details") {
                assert!(
                    details.get("manual_steps").is_some()
                        || details.get("reason").is_some()
                        || details.get("attempted_method").is_some(),
                    "Error details should explain the failure"
                );
            }
        } else if result.get("success").and_then(|v| v.as_bool()) == Some(true) {
            // If it succeeded (AppleScript worked), that's also OK
            assert!(
                result.get("method").is_some(),
                "Success should indicate which method worked"
            );
        } else {
            panic!("Unexpected response format: {:?}", result);
        }
    }
}
