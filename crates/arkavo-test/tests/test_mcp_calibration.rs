use arkavo_test::mcp::server::{McpTestServer, ToolRequest};
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_mcp_calibration_full_flow() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Testing MCP Calibration Full Flow ===\n");
    
    // Create MCP server - this should handle all IDB initialization
    println!("1. Creating MCP server...");
    let server = McpTestServer::new()?;
    println!("   ✓ MCP server created");
    
    // Get booted device - prefer iOS 18.x devices for better IDB compatibility
    println!("\n2. Getting booted device...");
    let device_result = server.call_tool(ToolRequest {
        tool_name: "device_management".to_string(),
        params: json!({
            "action": "list",
            "status": "booted"
        })
    }).await?;
    
    let devices = device_result.result["devices"].as_array()
        .ok_or("No devices found")?;
    let mut booted_devices: Vec<_> = devices.iter()
        .filter(|d| d["state"].as_str() == Some("Booted"))
        .collect();
    
    if booted_devices.is_empty() {
        return Err("No booted devices found".into());
    }
    
    // Prefer iOS 18.x devices over iOS 26.x for better IDB compatibility
    booted_devices.sort_by(|a, b| {
        let a_os = a["os_version"].as_str().unwrap_or("");
        let b_os = b["os_version"].as_str().unwrap_or("");
        // Prefer iOS 18.x
        if a_os.contains("18.") && !b_os.contains("18.") {
            std::cmp::Ordering::Less
        } else if !a_os.contains("18.") && b_os.contains("18.") {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    
    let device = booted_devices[0];
    let device_id = device["id"].as_str().ok_or("No device ID")?;
    let device_name = device["name"].as_str().unwrap_or("Unknown");
    let os_version = device["os_version"].as_str().unwrap_or("Unknown");
    
    println!("   ✓ Found booted device: {} ({}) - {}", device_name, device_id, os_version);
    
    // Check if ArkavoReference app is installed
    println!("\n3. Checking if ArkavoReference app is installed...");
    let app_check = std::process::Command::new("xcrun")
        .args(["simctl", "get_app_container", device_id, "com.arkavo.reference"])
        .output()?;
    
    if !app_check.status.success() {
        println!("   ℹ️  ArkavoReference app not installed. Installing...");
        
        let install_result = server.call_tool(ToolRequest {
            tool_name: "calibration_manager".to_string(),
            params: json!({
                "action": "install_reference_app",
                "device_id": device_id
            })
        }).await?;
        
        if install_result.result["success"].as_bool() == Some(true) {
            println!("   ✓ App installed successfully");
        } else {
            return Err("Failed to install reference app".into());
        }
    } else {
        println!("   ✓ ArkavoReference app already installed");
    }
    
    // Start calibration
    println!("\n4. Starting calibration...");
    let start_result = server.call_tool(ToolRequest {
        tool_name: "calibration_manager".to_string(),
        params: json!({
            "action": "start_calibration",
            "device_id": device_id
        })
    }).await?;
    
    if start_result.result["success"].as_bool() != Some(true) {
        return Err(format!("Failed to start calibration: {:?}", start_result.result).into());
    }
    
    let session_id = start_result.result["session_id"].as_str()
        .ok_or("No session ID returned")?;
    
    println!("   ✓ Calibration started with session ID: {}", session_id);
    
    // Monitor calibration progress
    println!("\n5. Monitoring calibration progress...");
    let mut last_status = String::new();
    let mut last_tap_count = 0;
    let max_checks = 60; // 60 seconds max
    
    for i in 0..max_checks {
        sleep(Duration::from_secs(1)).await;
        
        let status_result = server.call_tool(ToolRequest {
            tool_name: "calibration_manager".to_string(),
            params: json!({
                "action": "get_status",
                "session_id": session_id
            })
        }).await?;
        
        let status = status_result.result["status"].as_str().unwrap_or("unknown");
        let elapsed = status_result.result["elapsed_seconds"].as_u64().unwrap_or(0);
        let tap_count = status_result.result["tap_count"].as_u64().unwrap_or(0);
        let phase = status_result.result["phase"]["name"].as_str().unwrap_or("Unknown");
        
        // Print status update if changed
        if status != last_status || tap_count != last_tap_count {
            println!("   [{}s] Status: {} | Phase: {} | Taps: {}/5", 
                elapsed, status, phase, tap_count);
            
            // Print IDB status if available
            if let Some(idb_status) = status_result.result["idb_status"].as_object() {
                let companion_running = idb_status["companion_running"].as_bool().unwrap_or(false);
                let connected = idb_status["connected"].as_bool().unwrap_or(false);
                if let Some(error) = idb_status["last_error"].as_str() {
                    println!("      IDB: companion_running={}, connected={}, error={}", 
                        companion_running, connected, error);
                }
            }
            
            last_status = status.to_string();
            last_tap_count = tap_count;
        }
        
        // Check completion
        if status == "complete" {
            println!("\n   ✓ Calibration completed successfully!");
            break;
        } else if status.starts_with("failed") {
            println!("\n   ✗ Calibration failed: {}", status);
            return Err(format!("Calibration failed: {}", status).into());
        }
        
        // Timeout check
        if i == max_checks - 1 {
            println!("\n   ✗ Calibration timed out after {} seconds", max_checks);
            return Err("Calibration timeout".into());
        }
    }
    
    // Get calibration results
    println!("\n6. Getting calibration results...");
    let cal_result = server.call_tool(ToolRequest {
        tool_name: "calibration_manager".to_string(),
        params: json!({
            "action": "get_calibration",
            "device_id": device_id
        })
    }).await?;
    
    if cal_result.result["success"].as_bool() == Some(true) {
        println!("   ✓ Calibration data retrieved successfully");
        if let Some(config) = cal_result.result["config"].as_object() {
            println!("   Device: {} ({}x{})", 
                config["device_type"].as_str().unwrap_or("Unknown"),
                config["screen_size"]["width"].as_f64().unwrap_or(0.0),
                config["screen_size"]["height"].as_f64().unwrap_or(0.0)
            );
        }
    } else {
        println!("   ℹ️  No calibration data available");
    }
    
    println!("\n✅ MCP Calibration test completed!");
    
    Ok(())
}