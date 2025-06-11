use arkavo_test::mcp::server::{McpTestServer, ToolRequest};
use serde_json::json;

#[tokio::test]
async fn test_mcp_server_idb_integration() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Testing MCP Server with IDB ===\n");
    
    // Create MCP server
    let server = McpTestServer::new()?;
    
    // Test 1: List devices
    println!("1. Testing device_management list...");
    let device_result = server.call_tool(ToolRequest {
        tool_name: "device_management".to_string(),
        params: json!({
            "action": "list"
        })
    }).await?;
    
    println!("Device list success: {}", device_result.success);
    if let Some(devices) = device_result.result["devices"].as_array() {
        println!("Found {} devices", devices.len());
        for device in devices.iter().take(3) {
            println!("  - {} ({}) - {}", 
                device["name"].as_str().unwrap_or("Unknown"),
                device["id"].as_str().unwrap_or("Unknown"),
                device["state"].as_str().unwrap_or("Unknown")
            );
        }
    }
    
    // Test 2: Get booted devices
    println!("\n2. Testing device_management list booted devices...");
    let booted_result = server.call_tool(ToolRequest {
        tool_name: "device_management".to_string(),
        params: json!({
            "action": "list",
            "status": "booted"
        })
    }).await?;
    
    println!("Booted devices query success: {}", booted_result.success);
    if let Some(devices) = booted_result.result["devices"].as_array() {
        // Filter to only booted devices
        let booted_devices: Vec<_> = devices.iter()
            .filter(|d| d["state"].as_str() == Some("Booted"))
            .collect();
        
        println!("Found {} booted devices", booted_devices.len());
        
        // Test 3: Take a screenshot if we have a booted device
        if let Some(device) = booted_devices.first() {
            let device_id = device["id"].as_str().unwrap_or("unknown");
            let device_name = device["name"].as_str().unwrap_or("unknown");
            println!("\n3. Testing screenshot on device: {} ({})", device_name, device_id);
            
            let screenshot_result = server.call_tool(ToolRequest {
                tool_name: "screen_capture".to_string(),
                params: json!({
                    "device_id": device_id
                })
            }).await?;
            
            println!("Screenshot success: {}", screenshot_result.success);
            if let Some(path) = screenshot_result.result["path"].as_str() {
                println!("Screenshot saved to: {}", path);
                
                // Check if file exists
                if std::path::Path::new(path).exists() {
                    let metadata = std::fs::metadata(path)?;
                    println!("Screenshot file size: {} bytes", metadata.len());
                }
            }
            
            // Test 4: UI query
            println!("\n4. Testing UI query on device: {} ({})", device_name, device_id);
            let ui_result = server.call_tool(ToolRequest {
                tool_name: "ui_query".to_string(),
                params: json!({
                    "device_id": device_id,
                    "query_type": "visible_elements"
                })
            }).await?;
            
            println!("UI query success: {}", ui_result.success);
            if let Some(element_count) = ui_result.result["element_count"].as_u64() {
                println!("Found {} UI elements", element_count);
            }
            
            // Test 5: App management - list apps
            println!("\n5. Testing app management - list apps...");
            let app_list_result = server.call_tool(ToolRequest {
                tool_name: "app_management".to_string(),
                params: json!({
                    "action": "list",
                    "device_id": device_id
                })
            }).await?;
            
            println!("App list query success: {}", app_list_result.success);
            if let Some(apps) = app_list_result.result["apps"].as_array() {
                println!("Found {} apps installed", apps.len());
                // Show first few apps
                for app in apps.iter().take(3) {
                    if let Some(bundle_id) = app["bundle_id"].as_str() {
                        println!("  - {}", bundle_id);
                    }
                }
            }
        } else {
            println!("\nNo booted devices found. Skipping device-specific tests.");
        }
    }
    
    println!("\n✅ MCP Server IDB integration tests completed!");
    
    Ok(())
}