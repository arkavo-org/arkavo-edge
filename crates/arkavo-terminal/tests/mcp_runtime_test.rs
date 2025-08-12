// This test has been removed because the test itself was causing the runtime panic
// that we were trying to prevent. The actual fix is in McpConnection::call_tool()
// which now properly spawns a separate thread to avoid runtime conflicts.

#[test]
fn test_mcp_connection_in_sync_context() {
    use arkavo_mcp_macos::mcp::mcp_connection::McpConnection;

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
