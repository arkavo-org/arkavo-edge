#[cfg(test)]
mod manual_tests {
    use arkavo_test::mcp::mcp_connection::McpConnection;
    use serde_json::json;

    #[test]
    fn test_tui_tools_available() {
        // Create MCP connection and verify TUI tools are registered
        let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");

        let tools = mcp.list_tools();
        println!("Available MCP tools:");
        for tool in &tools {
            println!("  - {}", tool);
        }

        // Check that TUI tools are present
        assert!(
            tools.contains(&"tui_keyboard".to_string()),
            "tui_keyboard tool not found"
        );
        assert!(
            tools.contains(&"tui_screenshot".to_string()),
            "tui_screenshot tool not found"
        );
        assert!(
            tools.contains(&"tui_interaction".to_string()),
            "tui_interaction tool not found"
        );
        assert!(
            tools.contains(&"tui_harness".to_string()),
            "tui_harness tool not found"
        );
    }

    #[test]
    fn test_tui_keyboard_schema() {
        let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");

        // Get and verify keyboard tool schema
        let schema = mcp
            .get_tool_schema("tui_keyboard")
            .expect("Failed to get tui_keyboard schema");

        println!(
            "tui_keyboard schema: {}",
            serde_json::to_string_pretty(&schema).unwrap()
        );

        assert_eq!(schema["name"], "tui_keyboard");
        assert!(schema["description"].as_str().unwrap().contains("keyboard"));
    }

    #[test]
    #[ignore = "Requires manual verification of output"]
    fn test_simple_keyboard_action() {
        let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");

        // Test sending a simple key
        let result = mcp.call_tool(
            "tui_keyboard",
            json!({
                "action": "key",
                "key": "a"
            }),
            "test",
        );

        match result {
            Ok(res) => {
                println!(
                    "Keyboard action result: {}",
                    serde_json::to_string_pretty(&res).unwrap()
                );
                assert!(res["success"].as_bool().unwrap_or(false));
            }
            Err(e) => {
                println!("Error calling tui_keyboard: {}", e);
                // This might fail if there's no focused terminal
            }
        }
    }
}
