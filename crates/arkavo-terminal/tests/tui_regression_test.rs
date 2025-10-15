#![allow(clippy::disallowed_methods)]

use arkavo_mcp_tools::mcp_connection::McpConnection;
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
#[ignore = "Requires manual setup - run with --ignored flag"]
async fn test_arkavo_terminal_keyboard_shortcuts() {
    // Initialize MCP connection with TUI tools
    let mcp = McpConnection::new().expect("Failed to create MCP connection");

    // Start arkavo terminal in a test harness
    let session_id = "arkavo_test_session";
    let start_result = mcp
        .call_tool(
            "tui_harness",
            json!({
                "action": "start",
                "session_id": session_id,
                "command": "cargo",
                "args": ["run", "-p", "arkavo", "--", "terminal"],
                "working_dir": env!("CARGO_MANIFEST_DIR").replace("/crates/arkavo-terminal", "")
            }),
            "test",
        )
        .expect("Failed to start arkavo terminal");

    println!("Started arkavo terminal: {start_result:?}");

    // Wait for terminal to initialize
    sleep(Duration::from_secs(2)).await;

    // Test Tab cycling
    println!("Testing Tab key for model cycling...");
    let tab_result = mcp
        .call_tool(
            "tui_interaction",
            json!({
                "action": "shortcut_and_capture",
                "shortcut": "tab",
                "modifiers": [],
                "capture_delay_ms": 500
            }),
            "test",
        )
        .expect("Failed to send Tab key");

    assert!(tab_result["success"].as_bool().unwrap_or(false));
    println!("Tab key result: {tab_result:?}");

    // Test Ctrl+D for debug view
    println!("Testing Ctrl+D for debug view...");
    let ctrl_d_result = mcp
        .call_tool(
            "tui_interaction",
            json!({
                "action": "shortcut_and_capture",
                "shortcut": "d",
                "modifiers": ["ctrl"],
                "capture_delay_ms": 500
            }),
            "test",
        )
        .expect("Failed to send Ctrl+D");

    assert!(ctrl_d_result["success"].as_bool().unwrap_or(false));
    println!("Ctrl+D result: {ctrl_d_result:?}");

    // Test Ctrl+T for MCP tools dialog
    println!("Testing Ctrl+T for MCP tools dialog...");
    let ctrl_t_result = mcp
        .call_tool(
            "tui_interaction",
            json!({
                "action": "shortcut_and_capture",
                "shortcut": "t",
                "modifiers": ["ctrl"],
                "capture_delay_ms": 500
            }),
            "test",
        )
        .expect("Failed to send Ctrl+T");

    assert!(ctrl_t_result["success"].as_bool().unwrap_or(false));
    println!("Ctrl+T result: {ctrl_t_result:?}");

    // Test typing and verification
    println!("Testing text input...");
    let type_result = mcp
        .call_tool(
            "tui_interaction",
            json!({
                "action": "type_and_verify",
                "text": "Hello, TUI test!",
                "verify_delay_ms": 500,
                "expected_content": "Hello, TUI test!"
            }),
            "test",
        )
        .expect("Failed to type text");

    assert!(type_result["success"].as_bool().unwrap_or(false));
    assert!(
        type_result["verification_passed"]
            .as_bool()
            .unwrap_or(false)
    );
    println!("Type and verify result: {type_result:?}");

    // Clean up - stop the session
    let stop_result = mcp
        .call_tool(
            "tui_harness",
            json!({
                "action": "stop",
                "session_id": session_id
            }),
            "test",
        )
        .expect("Failed to stop session");

    assert!(stop_result["success"].as_bool().unwrap_or(false));
    println!("Session stopped: {stop_result:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Requires tui_harness tool which is not yet implemented"]
async fn test_tui_state_verification() {
    // Initialize MCP connection
    println!("Creating MCP connection...");
    let mcp = McpConnection::new().expect("Failed to create MCP connection");
    println!("MCP connection created successfully");

    // This test verifies we can capture and analyze terminal state
    let session_id = "state_test_session";

    // Start a simple command that outputs predictable text
    println!("Starting test session...");
    let start_result = mcp
        .call_tool(
            "tui_harness",
            json!({
                "action": "start",
                "session_id": session_id,
                "command": "echo",
                "args": ["Test output for TUI verification"]
            }),
            "test",
        )
        .expect("Failed to start echo command");

    assert!(start_result["success"].as_bool().unwrap_or(false));

    // Wait for output
    sleep(Duration::from_millis(500)).await;

    // Verify the output contains expected text
    let verify_result = mcp
        .call_tool(
            "tui_harness",
            json!({
                "action": "wait_for_output",
                "session_id": session_id,
                "expected_text": "Test output for TUI verification",
                "timeout_ms": 2000
            }),
            "test",
        )
        .expect("Failed to verify output");

    assert!(verify_result["success"].as_bool().unwrap_or(false));
    assert!(verify_result["found"].as_bool().unwrap_or(false));

    // Get the actual output
    let output_result = mcp
        .call_tool(
            "tui_harness",
            json!({
                "action": "get_output",
                "session_id": session_id,
                "last_n_lines": 10
            }),
            "test",
        )
        .expect("Failed to get output");

    assert!(output_result["success"].as_bool().unwrap_or(false));
    let output_lines = output_result["output"].as_array().unwrap();
    assert!(!output_lines.is_empty());

    println!("Captured output: {output_lines:?}");

    // Stop the session
    let _ = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "stop",
            "session_id": session_id
        }),
        "test",
    );
}
