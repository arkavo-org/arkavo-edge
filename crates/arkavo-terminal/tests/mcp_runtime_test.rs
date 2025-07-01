#[tokio::test]
async fn test_mcp_connection_in_async_context() {
    use arkavo_test::mcp::mcp_connection::McpConnection;

    // This test verifies that we can create an McpConnection
    // from within an async context without panicking
    let result = McpConnection::new_in_process();

    assert!(
        result.is_ok(),
        "Should be able to create McpConnection in async context"
    );

    let connection = result.unwrap();

    // Test that we can list tools
    let tools = connection.list_tools();
    assert!(!tools.is_empty(), "Should have some tools registered");
}

#[test]
fn test_mcp_connection_in_sync_context() {
    use arkavo_test::mcp::mcp_connection::McpConnection;

    // This test verifies that we can create an McpConnection
    // from a sync context (traditional behavior)
    let result = McpConnection::new_in_process();

    assert!(
        result.is_ok(),
        "Should be able to create McpConnection in sync context"
    );

    let connection = result.unwrap();

    // Test that we can list tools
    let tools = connection.list_tools();
    assert!(!tools.is_empty(), "Should have some tools registered");
}
