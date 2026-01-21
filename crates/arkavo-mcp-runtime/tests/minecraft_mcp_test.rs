//! Integration test for MCP client with Minecraft MCP server
//!
//! Requires:
//! - Minecraft server running on localhost:25565
//! - Node.js 20+ installed
//!
//! Run with: cargo test -p arkavo-mcp-runtime --test minecraft_mcp_test -- --ignored

#![allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
#![allow(clippy::significant_drop_tightening)] // server reference needed for tool calls

use arkavo_mcp_runtime::{McpRuntime, McpServerConfig};

#[tokio::test]
#[ignore = "Requires Minecraft server running on localhost:25565"]
async fn test_minecraft_mcp_connection() {
    // Create runtime
    let runtime = McpRuntime::new();

    // Configure the Minecraft MCP server (stdio transport)
    let config = McpServerConfig::stdio(
        "minecraft",
        "npx",
        vec![
            "-y".to_string(),
            "github:yuniko-software/minecraft-mcp-server".to_string(),
            "--host".to_string(),
            "localhost".to_string(),
            "--port".to_string(),
            "25565".to_string(),
            "--username".to_string(),
            "TestBot".to_string(),
        ],
    )
    .with_timeout(30000);

    // Connect to the MCP server
    let handle = runtime
        .add_server(config)
        .await
        .expect("Failed to connect to Minecraft MCP server");

    println!("Connected to MCP server: {}", handle.name());

    // List available tools (registered as proxy tools in the server)
    let server = runtime.server().await;
    let tool_names = server.list_tools().await;

    println!("Discovered {} tools:", tool_names.len());
    for name in &tool_names {
        println!("  - {name}");
    }

    // Verify we got some tools
    assert!(
        !tool_names.is_empty(),
        "Expected at least one tool from Minecraft MCP server"
    );

    // Cleanup
    runtime
        .remove_server("minecraft")
        .await
        .expect("Failed to remove server");

    println!("Test passed!");
}

#[tokio::test]
#[ignore = "Requires Minecraft server running on localhost:25565"]
async fn test_minecraft_tool_call() {
    let runtime = McpRuntime::new();

    let config = McpServerConfig::stdio(
        "minecraft",
        "npx",
        vec![
            "-y".to_string(),
            "github:yuniko-software/minecraft-mcp-server".to_string(),
            "--host".to_string(),
            "localhost".to_string(),
            "--port".to_string(),
            "25565".to_string(),
            "--username".to_string(),
            "TestBot2".to_string(),
        ],
    )
    .with_timeout(30000);

    runtime.add_server(config).await.expect("Failed to connect");

    // Get tool names
    let server = runtime.server().await;
    let tool_names = server.list_tools().await;

    println!("Available tools: {tool_names:?}");

    // Try calling get-position tool (Minecraft MCP uses kebab-case)
    if tool_names.iter().any(|n| n == "get-position") {
        println!("Found position tool: get-position");

        let request = arkavo_mcp_runtime::ToolRequest {
            tool_name: "get-position".to_string(),
            params: serde_json::json!({}),
        };

        let response = server.execute_tool(request).await;
        println!(
            "Tool response: success={}, result={}",
            response.success, response.result
        );
    } else {
        println!("No position tool found, available: {tool_names:?}");
    }

    runtime.shutdown().await.expect("Shutdown failed");
}
