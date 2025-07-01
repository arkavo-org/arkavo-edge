use arkavo_terminal::{LlmResponse, McpStatusUpdate};

#[test]
fn test_mcp_status_update_structure() {
    // Test that MCP status updates can be created and included in LlmResponse
    let mcp_status = McpStatusUpdate {
        available: false,
        error_message: Some("Test error message".to_string()),
        tool_count: 0,
    };

    let response = LlmResponse {
        task_id: uuid::Uuid::new_v4(),
        model_name: "system".to_string(),
        content: String::new(),
        is_streaming: false,
        is_complete: true,
        error: None,
        mcp_status: Some(mcp_status),
    };

    assert!(response.mcp_status.is_some());
    let status = response.mcp_status.unwrap();
    assert!(!status.available);
    assert_eq!(status.error_message, Some("Test error message".to_string()));
    assert_eq!(status.tool_count, 0);
}

#[test]
fn test_mcp_success_status() {
    let mcp_status = McpStatusUpdate {
        available: true,
        error_message: None,
        tool_count: 5,
    };

    let response = LlmResponse {
        task_id: uuid::Uuid::new_v4(),
        model_name: "system".to_string(),
        content: String::new(),
        is_streaming: false,
        is_complete: true,
        error: None,
        mcp_status: Some(mcp_status),
    };

    let status = response.mcp_status.unwrap();
    assert!(status.available);
    assert!(status.error_message.is_none());
    assert_eq!(status.tool_count, 5);
}
