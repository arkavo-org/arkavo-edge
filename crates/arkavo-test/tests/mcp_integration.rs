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
            "entity": "system"
        }),
    };

    let response = mcp_server.call_tool(request).await?;
    assert!(response.success);
    assert_eq!(response.tool_name, "query_state");
    assert!(response.result.get("state").is_some());

    Ok(())
}

#[tokio::test]
async fn test_mcp_server_snapshot() -> Result<(), TestError> {
    let harness = TestHarness::new()?;
    let mcp_server = harness.mcp_server();

    // Create snapshot
    let create_request = ToolRequest {
        tool_name: "snapshot".to_string(),
        params: json!({
            "action": "create",
            "name": "test_snapshot"
        }),
    };

    let create_response = mcp_server.call_tool(create_request).await?;
    assert!(create_response.success);
    assert!(create_response.result.get("snapshot_id").is_some());

    // List snapshots
    let list_request = ToolRequest {
        tool_name: "snapshot".to_string(),
        params: json!({
            "action": "list"
        }),
    };

    let list_response = mcp_server.call_tool(list_request).await?;
    assert!(list_response.success);
    assert!(list_response.result.get("snapshots").is_some());

    Ok(())
}

#[tokio::test]
async fn test_mcp_server_mutate_state() -> Result<(), TestError> {
    let harness = TestHarness::new()?;
    let mcp_server = harness.mcp_server();

    let request = ToolRequest {
        tool_name: "mutate_state".to_string(),
        params: json!({
            "entity": "user",
            "action": "create",
            "data": {
                "id": "test_user_1",
                "name": "Test User",
                "balance": 1000
            }
        }),
    };

    let response = mcp_server.call_tool(request).await?;
    assert!(response.success);
    assert_eq!(response.tool_name, "mutate_state");

    Ok(())
}

#[tokio::test]
async fn test_mcp_server_run_test() -> Result<(), TestError> {
    let harness = TestHarness::new()?;
    let mcp_server = harness.mcp_server();

    let request = ToolRequest {
        tool_name: "run_test".to_string(),
        params: json!({
            "test_name": "integration::mcp_server",
            "timeout": 5
        }),
    };

    let response = mcp_server.call_tool(request).await?;
    assert!(response.success);
    assert_eq!(response.tool_name, "run_test");
    assert_eq!(
        response.result.get("status").and_then(|v| v.as_str()),
        Some("passed")
    );

    Ok(())
}

#[tokio::test]
async fn test_mcp_server_invalid_tool() -> Result<(), TestError> {
    let harness = TestHarness::new()?;
    let mcp_server = harness.mcp_server();

    let request = ToolRequest {
        tool_name: "invalid_tool".to_string(),
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
