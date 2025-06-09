use arkavo_test::mcp::server::McpTestServer;
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_calibration_debug_flow() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("\n=== CALIBRATION DEBUG TEST ===\n");
    
    // Create MCP server
    let server = McpTestServer::new()?;
    
    // Step 1: Get booted device
    eprintln!("1. Getting booted device...");
    let device_result = server.call_tool("device_manager", json!({
        "action": "list",
        "status": "booted"
    })).await?;
    
    let devices = device_result["devices"].as_array()
        .ok_or("No devices found")?;
    let device = devices.first()
        .ok_or("No booted device found")?;
    let device_id = device["id"].as_str()
        .ok_or("No device ID")?;
    
    eprintln!("   Found device: {} ({})", device_id, device["name"].as_str().unwrap_or("Unknown"));
    
    // Step 2: Check if reference app is installed
    eprintln!("\n2. Checking if ArkavoReference app is installed...");
    let app_check_output = std::process::Command::new("xcrun")
        .args(["simctl", "get_app_container", device_id, "com.arkavo.reference"])
        .output()?;
    
    if !app_check_output.status.success() {
        eprintln!("   App not installed. Installing now...");
        let install_result = server.call_tool("calibration_manager", json!({
            "action": "install_reference_app",
            "device_id": device_id
        })).await?;
        eprintln!("   Install result: {}", serde_json::to_string_pretty(&install_result)?);
    } else {
        eprintln!("   App already installed");
    }
    
    // Step 3: Start calibration
    eprintln!("\n3. Starting calibration...");
    let start_result = server.call_tool("calibration_manager", json!({
        "action": "start_calibration",
        "device_id": device_id
    })).await?;
    
    eprintln!("   Start result: {}", serde_json::to_string_pretty(&start_result)?);
    
    let session_id = start_result["session_id"].as_str()
        .ok_or("No session ID returned")?;
    
    // Step 4: Monitor calibration progress
    eprintln!("\n4. Monitoring calibration progress...");
    let mut last_status = String::new();
    let mut stuck_counter = 0;
    let max_stuck_checks = 10;
    
    for i in 0..30 {  // Check for up to 30 seconds
        sleep(Duration::from_secs(1)).await;
        
        let status_result = server.call_tool("calibration_manager", json!({
            "action": "get_status",
            "session_id": session_id
        })).await?;
        
        let status = status_result["status"].as_str().unwrap_or("unknown");
        let elapsed = status_result["elapsed_seconds"].as_u64().unwrap_or(0);
        let app_running = status_result["app_running"].as_bool().unwrap_or(false);
        
        eprintln!("   [{}s] Status: {} (elapsed: {}s, app_running: {})", 
            i + 1, status, elapsed, app_running);
        
        // Check if stuck
        if status == last_status && status == "validating" {
            stuck_counter += 1;
            if stuck_counter >= max_stuck_checks {
                eprintln!("\n   WARNING: Calibration stuck in 'validating' for {} seconds", stuck_counter);
                eprintln!("   This suggests taps are being executed but not detected by the app");
                
                // Try to check what's happening
                eprintln!("\n   Debugging stuck calibration:");
                
                // Check if app is actually running
                let ps_output = std::process::Command::new("xcrun")
                    .args(["simctl", "spawn", device_id, "ps", "aux"])
                    .output()?;
                
                if ps_output.status.success() {
                    let ps_str = String::from_utf8_lossy(&ps_output.stdout);
                    if ps_str.contains("ArkavoReference") {
                        eprintln!("   ✓ ArkavoReference process is running");
                    } else {
                        eprintln!("   ✗ ArkavoReference process NOT found");
                    }
                }
                
                // Check app Documents directory
                let docs_output = std::process::Command::new("xcrun")
                    .args(["simctl", "get_app_container", device_id, "com.arkavo.reference", "data"])
                    .output()?;
                
                if docs_output.status.success() {
                    let container_path = String::from_utf8_lossy(&docs_output.stdout).trim().to_string();
                    eprintln!("   App container: {}", container_path);
                    
                    // List Documents directory
                    let docs_path = format!("{}/Documents", container_path);
                    let ls_output = std::process::Command::new("ls")
                        .args(["-la", &docs_path])
                        .output()?;
                    
                    if ls_output.status.success() {
                        eprintln!("   Documents contents:\n{}", 
                            String::from_utf8_lossy(&ls_output.stdout));
                    }
                }
            }
        } else {
            stuck_counter = 0;
        }
        
        last_status = status.to_string();
        
        if status == "complete" {
            eprintln!("\n   ✓ Calibration completed successfully!");
            break;
        } else if status.starts_with("failed") {
            eprintln!("\n   ✗ Calibration failed: {}", status);
            break;
        }
    }
    
    // Step 5: Get final calibration data
    eprintln!("\n5. Getting calibration data...");
    match server.call_tool("calibration_manager", json!({
        "action": "get_calibration",
        "device_id": device_id
    })).await {
        Ok(cal_data) => {
            if cal_data["success"].as_bool() == Some(true) {
                eprintln!("   ✓ Calibration data retrieved successfully");
                eprintln!("   Config: {}", serde_json::to_string_pretty(&cal_data["config"])?);
            } else {
                eprintln!("   ✗ No calibration data found");
            }
        }
        Err(e) => {
            eprintln!("   ✗ Failed to get calibration data: {}", e);
        }
    }
    
    eprintln!("\n=== TEST COMPLETE ===\n");
    
    Ok(())
}