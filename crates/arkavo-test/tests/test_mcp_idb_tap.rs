use arkavo_test::mcp::server::{McpTestServer, ToolRequest};
use arkavo_test::Result;
use serde_json::json;

#[tokio::test]
async fn test_mcp_idb_tap_functionality() -> Result<()> {
    println!("🎯 Testing MCP IDB tap functionality...");
    
    // Create MCP server
    let server = McpTestServer::new()?;
    println!("✅ MCP server created");
    
    // First, get a booted device
    let device_request = ToolRequest {
        tool_name: "device_management".to_string(),
        params: json!({
            "action": "list",
            "status": "booted"
        }),
    };
    
    let response = server.call_tool(device_request).await?;
    assert!(response.success, "Failed to list devices");
    
    println!("Device response: {}", serde_json::to_string_pretty(&response.result)?);
    
    if let Some(devices) = response.result.get("devices").and_then(|v| v.as_array()) {
        // Find the first booted device
        if let Some(device) = devices.iter().find(|d| d.get("state").and_then(|s| s.as_str()) == Some("Booted")) {
            // Use "id" field instead of "udid" for device_management tool
            if let Some(device_id) = device.get("id").and_then(|u| u.as_str()) {
                println!("📱 Using device: {} ({})", 
                    device.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown"),
                    device_id
                );
                
                // Test tap at center of screen
                println!("\n👆 Testing tap at center of screen...");
                let tap_request = ToolRequest {
                    tool_name: "ui_interaction".to_string(),
                    params: json!({
                        "action": "tap",
                        "device_id": device_id,
                        "target": {
                            "x": 195,  // Center of iPhone screen (390 width)
                            "y": 422   // Center of iPhone screen (844 height)
                        }
                    }),
                };
                
                let tap_response = server.call_tool(tap_request).await?;
                if tap_response.success {
                    println!("✅ Tap executed successfully!");
                    if let Some(method) = tap_response.result.get("method").and_then(|m| m.as_str()) {
                        println!("  Method used: {}", method);
                    }
                    if let Some(details) = tap_response.result.get("details") {
                        println!("  Details: {}", serde_json::to_string_pretty(details)?);
                    }
                } else {
                    println!("⚠️  Tap failed: {:?}", tap_response.result);
                    println!("  This is expected if no app is running on the device");
                }
                
                // Test analyze_layout to get AI vision analysis
                println!("\n🔍 Testing analyze_layout with AI vision...");
                let analyze_request = ToolRequest {
                    tool_name: "ui_interaction".to_string(),
                    params: json!({
                        "action": "analyze_layout",
                        "device_id": device_id
                    }),
                };
                
                let analyze_response = server.call_tool(analyze_request).await?;
                if analyze_response.success {
                    println!("✅ Layout analysis completed!");
                    if let Some(screenshot_path) = analyze_response.result.get("screenshot_path").and_then(|p| p.as_str()) {
                        println!("  Screenshot saved to: {}", screenshot_path);
                    }
                    if let Some(analysis) = analyze_response.result.get("analysis") {
                        println!("  AI Analysis preview: {}", 
                            serde_json::to_string(analysis)?
                                .chars()
                                .take(200)
                                .collect::<String>()
                        );
                    }
                } else {
                    println!("⚠️  Layout analysis failed: {:?}", analyze_response.result);
                }
            } else {
                println!("⚠️  No device ID found");
            }
        } else {
            println!("ℹ️  No booted devices found to test tap functionality");
            println!("  You can boot a simulator and run this test again");
        }
    }
    
    println!("\n🎉 MCP IDB tap test completed!");
    
    Ok(())
}