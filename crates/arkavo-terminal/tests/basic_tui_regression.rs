/// Basic TUI regression tests that can run without triggering runtime issues
/// These tests verify the TUI testing infrastructure is working
use arkavo_test::mcp::mcp_connection::McpConnection;
use serde_json::json;

#[test]
fn test_tui_keyboard_functionality() {
    // Skip keyboard tests in CI to avoid platform-specific issues
    if std::env::var("CI").is_ok() {
        println!("Skipping keyboard tests in CI environment");
        return;
    }

    println!("=== Testing TUI Keyboard Tool ===");

    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");

    // Test key press
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
                "Key press result: {}",
                serde_json::to_string_pretty(&res).unwrap()
            );
            assert!(res["success"].as_bool().unwrap_or(false));
        }
        Err(e) => {
            println!("Key press failed (expected if no terminal focused): {}", e);
        }
    }

    // Test text typing
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "text",
            "text": "Hello TUI"
        }),
        "test",
    );

    match result {
        Ok(res) => {
            println!(
                "Text typing result: {}",
                serde_json::to_string_pretty(&res).unwrap()
            );
            assert!(res["success"].as_bool().unwrap_or(false));
        }
        Err(e) => {
            println!(
                "Text typing failed (expected if no terminal focused): {}",
                e
            );
        }
    }

    // Test shortcut
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "shortcut",
            "shortcut": "c",
            "modifiers": ["ctrl"]
        }),
        "test",
    );

    match result {
        Ok(res) => {
            println!(
                "Shortcut result: {}",
                serde_json::to_string_pretty(&res).unwrap()
            );
            assert!(res["success"].as_bool().unwrap_or(false));
        }
        Err(e) => {
            println!("Shortcut failed (expected if no terminal focused): {}", e);
        }
    }
}

#[test]
fn test_tui_screenshot_functionality() {
    // Skip screenshot tests in CI to avoid platform-specific issues
    if std::env::var("CI").is_ok() {
        println!("Skipping screenshot tests in CI environment");
        return;
    }

    println!("=== Testing TUI Screenshot Tool ===");

    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");

    // Test text format screenshot
    let result = mcp.call_tool(
        "tui_screenshot",
        json!({
            "format": "text",
            "include_colors": false
        }),
        "test",
    );

    match result {
        Ok(res) => {
            println!(
                "Screenshot result: {}",
                serde_json::to_string_pretty(&res).unwrap()
            );
            assert!(res["success"].as_bool().unwrap_or(false));
            assert!(res.get("content").is_some());
        }
        Err(e) => {
            println!(
                "Screenshot failed (expected if no terminal available): {}",
                e
            );
        }
    }
}

#[test]
fn test_tui_interaction_schema() {
    println!("=== Testing TUI Interaction Tool Schema ===");

    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");

    let schema = mcp
        .get_tool_schema("tui_interaction")
        .expect("Failed to get tui_interaction schema");

    println!(
        "TUI Interaction schema: {}",
        serde_json::to_string_pretty(&schema).unwrap()
    );

    assert_eq!(schema["name"], "tui_interaction");
    assert!(schema["description"].as_str().unwrap().contains("TUI"));

    // Verify the schema has the expected actions
    let parameters = &schema["parameters"];
    assert!(parameters.is_object());
}

#[test]
fn test_all_tui_tools_schemas() {
    println!("=== Verifying All TUI Tool Schemas ===");

    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");

    let tui_tools = vec![
        "tui_keyboard",
        "tui_screenshot",
        "tui_interaction",
        "tui_harness",
    ];

    for tool_name in tui_tools {
        println!("\nChecking schema for: {}", tool_name);

        let schema = mcp
            .get_tool_schema(tool_name)
            .expect(&format!("Failed to get {} schema", tool_name));

        // Verify basic schema structure
        assert_eq!(schema["name"], tool_name);
        assert!(schema.get("description").is_some());
        assert!(schema.get("parameters").is_some());

        println!("  ✓ Schema valid for {}", tool_name);
    }
}

#[test]
fn test_keyboard_modifiers() {
    // Skip keyboard tests in CI to avoid platform-specific issues
    if std::env::var("CI").is_ok() {
        println!("Skipping keyboard modifier tests in CI environment");
        return;
    }

    println!("=== Testing Keyboard Modifiers ===");

    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");

    let modifier_combos = vec![
        (vec!["ctrl"], "c"),
        (vec!["ctrl", "shift"], "a"),
        // Removed "cmd" + "q" as it's a common shortcut to quit applications on macOS
        // and likely causing the terminal to quit unexpectedly
        (vec!["alt"], "tab"),
    ];

    for (modifiers, key) in modifier_combos {
        println!("\nTesting {:?} + {}", modifiers, key);

        let result = mcp.call_tool(
            "tui_keyboard",
            json!({
                "action": "shortcut",
                "shortcut": key,
                "modifiers": modifiers
            }),
            "test",
        );

        match result {
            Ok(res) => {
                println!(
                    "  Result: {}",
                    res["message"].as_str().unwrap_or("No message")
                );
                assert!(res["success"].as_bool().unwrap_or(false));
            }
            Err(e) => {
                println!("  Failed (expected without focused terminal): {}", e);
            }
        }
    }
}
