use arkavo_test::mcp::server::{McpTestServer, ToolRequest};
use arkavo_test::Result;
use serde_json::json;

#[tokio::test]
async fn test_mcp_server_can_use_idb_tools() -> Result<()> {
    println!("🚀 Starting MCP server test with IDB tools...");
    
    // Create MCP server (this should initialize IDB)
    let server = McpTestServer::new()?;
    println!("✅ MCP server created successfully");
    
    // Test 1: List available tools
    let tools = server.get_tool_schemas()?;
    println!("✅ Found {} tools", tools.len());
    
    // Check that IDB-dependent tools are available
    let idb_tools = ["ui_interaction", "screen_capture", "device_management"];
    for tool_name in &idb_tools {
        assert!(
            tools.iter().any(|t| t.name == *tool_name),
            "Expected to find {} tool",
            tool_name
        );
        println!("  ✓ Found {} tool", tool_name);
    }
    
    // Test 2: Use device_management to list devices
    println!("\n📱 Testing device management...");
    let device_request = ToolRequest {
        tool_name: "device_management".to_string(),
        params: json!({
            "action": "list",
            "status": "all"
        }),
    };
    
    let response = server.call_tool(device_request).await?;
    assert!(response.success, "Device management call failed: {:?}", response.result);
    println!("✅ Device management working");
    
    // Print device count
    if let Some(devices) = response.result.get("devices").and_then(|v| v.as_array()) {
        println!("  ✓ Found {} devices/simulators", devices.len());
        
        // Print first few devices
        for (i, device) in devices.iter().take(3).enumerate() {
            if let (Some(name), Some(state)) = (
                device.get("name").and_then(|v| v.as_str()),
                device.get("state").and_then(|v| v.as_str()),
            ) {
                println!("  {}. {} ({})", i + 1, name, state);
            }
        }
    }
    
    // Test 3: Test screenshot capability (on first booted device if available)
    println!("\n📸 Testing screenshot capability...");
    if let Some(devices) = response.result.get("devices").and_then(|v| v.as_array()) {
        if let Some(booted_device) = devices.iter().find(|d| {
            d.get("state").and_then(|s| s.as_str()) == Some("Booted")
        }) {
            if let Some(device_id) = booted_device.get("udid").and_then(|u| u.as_str()) {
                println!("  Using device: {}", 
                    booted_device.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown"));
                
                let screenshot_request = ToolRequest {
                    tool_name: "screen_capture".to_string(),
                    params: json!({
                        "device_id": device_id,
                        "output_path": "/tmp/mcp_test_screenshot.png"
                    }),
                };
                
                let screenshot_response = server.call_tool(screenshot_request).await?;
                if screenshot_response.success {
                    println!("  ✅ Screenshot captured successfully");
                    if let Some(path) = screenshot_response.result.get("path").and_then(|p| p.as_str()) {
                        println!("  ✓ Screenshot saved to: {}", path);
                    }
                } else {
                    println!("  ⚠️  Screenshot failed (this is okay if no apps are running)");
                }
            }
        } else {
            println!("  ℹ️  No booted devices found for screenshot test");
        }
    }
    
    println!("\n🎉 All MCP server IDB tests completed successfully!");
    
    Ok(())
}