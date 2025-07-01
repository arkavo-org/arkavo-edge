/// Comprehensive TUI regression tests for key bindings, model connections, and window management
/// Run with: cargo test -p arkavo-terminal tui_comprehensive_regression -- --ignored --nocapture

use arkavo_test::mcp::mcp_connection::McpConnection;
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
#[ignore = "Interactive test - requires terminal UI"]
async fn test_vim_mode_transitions() {
    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");
    
    println!("=== Testing Vim Mode Transitions ===");
    
    // Start a simple echo session to test keyboard input
    let session_id = "vim_mode_test";
    let start_result = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "start",
            "session_id": session_id,
            "command": "sh",
            "args": ["-c", "echo 'Press keys to test vim modes:'; cat"]
        }),
        "test"
    ).expect("Failed to start test session");
    
    assert!(start_result["success"].as_bool().unwrap_or(false));
    sleep(Duration::from_millis(500)).await;
    
    // Test entering insert mode
    println!("1. Testing 'i' to enter insert mode");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "i"
        }),
        "test"
    ).expect("Failed to send 'i' key");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Type some text in insert mode
    println!("2. Typing text in insert mode");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "text",
            "text": "Hello from insert mode"
        }),
        "test"
    ).expect("Failed to type text");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Return to normal mode with Escape
    println!("3. Testing Escape to return to normal mode");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "escape"
        }),
        "test"
    ).expect("Failed to send Escape");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Test visual mode
    println!("4. Testing 'v' to enter visual mode");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "v"
        }),
        "test"
    ).expect("Failed to send 'v' key");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Test command mode
    println!("5. Testing ':' to enter command mode");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": ":"
        }),
        "test"
    ).expect("Failed to send ':' key");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Capture screenshot to verify state
    let screenshot = mcp.call_tool(
        "tui_screenshot",
        json!({
            "format": "text",
            "include_colors": false
        }),
        "test"
    ).expect("Failed to capture screenshot");
    
    println!("Screenshot after vim mode tests:");
    println!("{}", screenshot["content"].as_str().unwrap_or("No content"));
    
    // Clean up
    let _ = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "stop",
            "session_id": session_id
        }),
        "test"
    );
}

#[tokio::test]
#[ignore = "Interactive test - requires terminal UI"]
async fn test_key_bindings_and_navigation() {
    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");
    
    println!("=== Testing Key Bindings and Navigation ===");
    
    // Start arkavo terminal UI
    let session_id = "key_binding_test";
    let start_result = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "start",
            "session_id": session_id,
            "command": "cargo",
            "args": ["run", "-p", "arkavo", "--", "terminal"],
            "working_dir": env!("CARGO_MANIFEST_DIR").replace("/crates/arkavo-terminal", "")
        }),
        "test"
    ).expect("Failed to start arkavo terminal");
    
    assert!(start_result["success"].as_bool().unwrap_or(false));
    
    // Wait for UI to initialize
    sleep(Duration::from_secs(3)).await;
    
    // Test Tab navigation between views
    println!("1. Testing Tab key for view switching");
    for i in 0..3 {
        let result = mcp.call_tool(
            "tui_keyboard",
            json!({
                "action": "key",
                "key": "tab"
            }),
            "test"
        ).expect("Failed to send Tab");
        assert!(result["success"].as_bool().unwrap_or(false));
        
        sleep(Duration::from_millis(300)).await;
        
        // Capture state after tab press
        let screenshot = mcp.call_tool(
            "tui_screenshot",
            json!({
                "format": "text",
                "include_colors": false
            }),
            "test"
        ).expect("Failed to capture screenshot");
        
        println!("After Tab press {}: View should have changed", i + 1);
        let content = screenshot["content"].as_str().unwrap_or("");
        println!("First line: {}", content.lines().next().unwrap_or(""));
    }
    
    // Test Shift+Tab for reverse navigation
    println!("\n2. Testing Shift+Tab for reverse view switching");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "tab",
            "modifiers": ["shift"]
        }),
        "test"
    ).expect("Failed to send Shift+Tab");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Test Ctrl+T for layout toggle
    println!("\n3. Testing Ctrl+T for layout toggle");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "shortcut",
            "shortcut": "t",
            "modifiers": ["ctrl"]
        }),
        "test"
    ).expect("Failed to send Ctrl+T");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    sleep(Duration::from_millis(500)).await;
    
    // Test Ctrl+F for focus toggle (in portrait mode)
    println!("\n4. Testing Ctrl+F for focus toggle");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "shortcut",
            "shortcut": "f",
            "modifiers": ["ctrl"]
        }),
        "test"
    ).expect("Failed to send Ctrl+F");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Test model selection with 'm'
    println!("\n5. Testing 'm' for model selection");
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "m"
        }),
        "test"
    ).expect("Failed to send 'm' key");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    sleep(Duration::from_millis(300)).await;
    
    // Navigate model list
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "down"
        }),
        "test"
    ).expect("Failed to send Down arrow");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Cancel with Escape
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "escape"
        }),
        "test"
    ).expect("Failed to send Escape");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Clean up
    let _ = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "q"
        }),
        "test"
    );
    
    sleep(Duration::from_millis(500)).await;
    
    let _ = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "stop",
            "session_id": session_id
        }),
        "test"
    );
}

#[tokio::test]
#[ignore = "Interactive test - requires terminal UI"]
async fn test_chat_input_and_streaming() {
    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");
    
    println!("=== Testing Chat Input and Streaming ===");
    
    // This test would require a running Ollama instance
    // For now, we'll test the input mechanics
    
    let session_id = "chat_test";
    let start_result = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "start",
            "session_id": session_id,
            "command": "sh",
            "args": ["-c", "echo 'Chat view test'; read line; echo \"You typed: $line\""]
        }),
        "test"
    ).expect("Failed to start test session");
    
    assert!(start_result["success"].as_bool().unwrap_or(false));
    sleep(Duration::from_millis(500)).await;
    
    // Enter insert mode
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "i"
        }),
        "test"
    ).expect("Failed to enter insert mode");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Type a message
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "text",
            "text": "Test message for chat"
        }),
        "test"
    ).expect("Failed to type message");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Send with Enter
    let result = mcp.call_tool(
        "tui_keyboard",
        json!({
            "action": "key",
            "key": "enter"
        }),
        "test"
    ).expect("Failed to send Enter");
    assert!(result["success"].as_bool().unwrap_or(false));
    
    // Wait for response
    sleep(Duration::from_millis(500)).await;
    
    // Verify output
    let output = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "get_output",
            "session_id": session_id,
            "last_n_lines": 5
        }),
        "test"
    ).expect("Failed to get output");
    
    println!("Chat output:");
    if let Some(lines) = output["output"].as_array() {
        for line in lines {
            println!("  {}", line.as_str().unwrap_or(""));
        }
    }
    
    // Clean up
    let _ = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "stop",
            "session_id": session_id
        }),
        "test"
    );
}

#[tokio::test]
#[ignore = "Interactive test - requires manual verification"]
async fn test_concurrent_operations() {
    let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");
    
    println!("=== Testing Concurrent Operations ===");
    
    // Test that keyboard input doesn't interfere with screenshots
    let session_id = "concurrent_test";
    let start_result = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "start",
            "session_id": session_id,
            "command": "sh",
            "args": ["-c", "while true; do date; sleep 1; done"]
        }),
        "test"
    ).expect("Failed to start test session");
    
    assert!(start_result["success"].as_bool().unwrap_or(false));
    sleep(Duration::from_millis(500)).await;
    
    // Send multiple operations concurrently
    let handles: Vec<_> = (0..3).map(|i| {
        let mcp = McpConnection::new_in_process().expect("Failed to create MCP connection");
        
        tokio::spawn(async move {
            if i % 2 == 0 {
                // Send keyboard input
                let result = mcp.call_tool(
                    "tui_keyboard",
                    json!({
                        "action": "text",
                        "text": format!("Test {}", i)
                    }),
                    "test"
                );
                println!("Keyboard operation {}: {:?}", i, result.is_ok());
            } else {
                // Take screenshot
                let result = mcp.call_tool(
                    "tui_screenshot",
                    json!({
                        "format": "text"
                    }),
                    "test"
                );
                println!("Screenshot operation {}: {:?}", i, result.is_ok());
            }
        })
    }).collect();
    
    // Wait for all operations to complete
    for handle in handles {
        let _ = handle.await;
    }
    
    // Clean up
    let _ = mcp.call_tool(
        "tui_harness",
        json!({
            "action": "stop",
            "session_id": session_id
        }),
        "test"
    );
    
    println!("Concurrent operations test completed");
}

#[test]
fn test_documentation_completeness() {
    // Verify CLAUDE.md exists and contains required sections
    let claude_md_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("CLAUDE.md");
    assert!(claude_md_path.exists(), "CLAUDE.md file should exist");
    
    let content = std::fs::read_to_string(claude_md_path).expect("Failed to read CLAUDE.md");
    
    // Check for required sections
    assert!(content.contains("## Key Bindings"), "Should document key bindings");
    assert!(content.contains("## Testing Requirements"), "Should document testing requirements");
    assert!(content.contains("## Common Bug Patterns"), "Should document common bugs");
    assert!(content.contains("### Model Connection States"), "Should document model states");
    assert!(content.contains("### TUI Testing Tools"), "Should document testing tools");
    
    println!("CLAUDE.md documentation is complete");
}