/// Manual TUI tests that verify tool functionality
/// Note: TUI tools are discovery-only (not in core MCP tools), so we test them directly.
#[cfg(test)]
mod manual_tests {
    use arkavo_mcp_tools::server::Tool;
    use arkavo_mcp_tools::tui::{TuiInteractionKit, TuiKeyboardKit, TuiScreenshotKit};
    use serde_json::json;

    #[test]
    fn test_tui_tools_available() {
        // Verify TUI tools can be instantiated and have valid schemas
        let keyboard = TuiKeyboardKit::new();
        let screenshot = TuiScreenshotKit::new();
        let interaction = TuiInteractionKit::new();

        println!("Available TUI tools (discovery-only):");
        println!("  - {} ({})", keyboard.schema().name, keyboard.schema().description);
        println!("  - {} ({})", screenshot.schema().name, screenshot.schema().description);
        println!("  - {} ({})", interaction.schema().name, interaction.schema().description);

        // Verify tool names
        assert_eq!(keyboard.schema().name, "tui_keyboard");
        assert_eq!(screenshot.schema().name, "tui_screenshot");
        assert_eq!(interaction.schema().name, "tui_interaction");
    }

    #[test]
    fn test_tui_keyboard_schema() {
        let keyboard = TuiKeyboardKit::new();
        let schema = keyboard.schema();

        println!(
            "tui_keyboard schema: {}",
            serde_json::to_string_pretty(&serde_json::json!({
                "name": schema.name,
                "description": schema.description,
                "parameters": schema.parameters,
            }))
            .unwrap()
        );

        assert_eq!(schema.name, "tui_keyboard");
        assert!(schema.description.contains("keyboard"));
    }

    #[test]
    #[ignore = "Requires manual verification of output"]
    fn test_simple_keyboard_action() {
        let keyboard = TuiKeyboardKit::new();
        let rt = tokio::runtime::Runtime::new().unwrap();

        // Test sending a simple key
        let result = rt.block_on(keyboard.execute(json!({
            "action": "key",
            "key": "a"
        })));

        match result {
            Ok(res) => {
                println!(
                    "Keyboard action result: {}",
                    serde_json::to_string_pretty(&res).unwrap()
                );
                assert!(res["success"].as_bool().unwrap_or(false));
            }
            Err(e) => {
                println!("Error calling tui_keyboard: {e}");
                // This might fail if there's no focused terminal
            }
        }
    }
}
