use arkavo_test::mcp::server::{McpTestServer, ToolRequest};
use serde_json::json;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_calibration_stuck_debug() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("\n=== CALIBRATION STUCK DEBUG TEST ===\n");
    
    // Create MCP server
    let server = McpTestServer::new()?;
    
    // Step 1: Get booted device
    eprintln!("1. Getting booted device...");
    let device_request = ToolRequest {
        tool_name: "device_manager".to_string(),
        params: json!({
            "action": "list",
            "status": "booted"
        }),
    };
    let device_response = server.call_tool(device_request).await?;
    let device_result = device_response.result;
    
    let devices = device_result["devices"].as_array()
        .ok_or("No devices found")?;
    let device = devices.first()
        .ok_or("No booted device found")?;
    let device_id = device["id"].as_str()
        .ok_or("No device ID")?;
    
    eprintln!("   Found device: {} ({})", device_id, device["name"].as_str().unwrap_or("Unknown"));
    
    // Step 2: Install reference app if needed
    eprintln!("\n2. Installing ArkavoReference app...");
    let install_request = ToolRequest {
        tool_name: "calibration_manager".to_string(),
        params: json!({
            "action": "install_reference_app",
            "device_id": device_id
        }),
    };
    let install_response = server.call_tool(install_request).await?;
    eprintln!("   Install response: {:?}", install_response.result);
    
    // Step 3: Launch app directly to see if it works
    eprintln!("\n3. Launching ArkavoReference app directly...");
    let launch_output = std::process::Command::new("xcrun")
        .args(["simctl", "launch", device_id, "com.arkavo.reference"])
        .output()?;
    
    if launch_output.status.success() {
        eprintln!("   ✓ App launched successfully");
    } else {
        eprintln!("   ✗ App launch failed: {}", String::from_utf8_lossy(&launch_output.stderr));
    }
    
    // Wait for app to start
    sleep(Duration::from_secs(3)).await;
    
    // Step 4: Test tap directly with idb_companion
    eprintln!("\n4. Testing direct tap with idb_companion...");
    
    // Get screen dimensions
    let dimensions_output = std::process::Command::new("xcrun")
        .args(["simctl", "list", "devicetypes", "--json"])
        .output()?;
    
    let mut screen_width = 390.0;
    let mut screen_height = 844.0;
    
    if dimensions_output.status.success() {
        if let Ok(types_json) = serde_json::from_slice::<serde_json::Value>(&dimensions_output.stdout) {
            eprintln!("   Device types available, looking for screen dimensions...");
        }
    }
    
    // Calculate 20%, 20% position
    let test_x = screen_width * 0.2;
    let test_y = screen_height * 0.2;
    
    eprintln!("   Testing tap at ({}, {}) - 20%, 20% of screen", test_x, test_y);
    
    // First try simctl io tap
    eprintln!("\n   a) Testing with simctl io tap...");
    let simctl_tap = std::process::Command::new("xcrun")
        .args(["simctl", "io", device_id, "tap", &test_x.to_string(), &test_y.to_string()])
        .output()?;
    
    if simctl_tap.status.success() {
        eprintln!("      ✓ simctl tap succeeded");
    } else {
        eprintln!("      ✗ simctl tap failed: {}", String::from_utf8_lossy(&simctl_tap.stderr));
    }
    
    sleep(Duration::from_millis(500)).await;
    
    // Check if idb_companion exists
    eprintln!("\n   b) Checking for idb_companion...");
    let idb_check = std::process::Command::new("which")
        .args(["idb_companion"])
        .output()?;
    
    if idb_check.status.success() {
        let idb_path = String::from_utf8_lossy(&idb_check.stdout).trim().to_string();
        eprintln!("      Found idb_companion at: {}", idb_path);
        
        eprintln!("\n   c) Testing with idb_companion tap...");
        let idb_tap = std::process::Command::new(&idb_path)
            .args([
                "--udid", device_id,
                "--only", "simulator",
                "ui", "tap",
                &test_x.to_string(),
                &test_y.to_string()
            ])
            .output()?;
        
        if idb_tap.status.success() {
            eprintln!("      ✓ idb_companion tap succeeded");
        } else {
            eprintln!("      ✗ idb_companion tap failed: {}", String::from_utf8_lossy(&idb_tap.stderr));
        }
    } else {
        eprintln!("      ✗ idb_companion not found in PATH");
    }
    
    // Step 5: Check app documents directory
    eprintln!("\n5. Checking app documents directory...");
    let docs_output = std::process::Command::new("xcrun")
        .args(["simctl", "get_app_container", device_id, "com.arkavo.reference", "data"])
        .output()?;
    
    if docs_output.status.success() {
        let container_path = String::from_utf8_lossy(&docs_output.stdout).trim().to_string();
        eprintln!("   App container: {}", container_path);
        
        let docs_path = format!("{}/Documents", container_path);
        let ls_output = std::process::Command::new("ls")
            .args(["-la", &docs_path])
            .output()?;
        
        if ls_output.status.success() {
            eprintln!("   Documents directory contents:");
            eprintln!("{}", String::from_utf8_lossy(&ls_output.stdout));
            
            // Check for calibration results
            let cal_file = format!("{}/calibration_results.json", docs_path);
            if std::path::Path::new(&cal_file).exists() {
                eprintln!("\n   ✓ Found calibration_results.json!");
                let contents = std::fs::read_to_string(&cal_file)?;
                eprintln!("   Contents: {}", contents);
            } else {
                eprintln!("\n   ✗ No calibration_results.json found");
            }
        }
    }
    
    // Step 6: Start actual calibration
    eprintln!("\n6. Starting calibration through MCP...");
    let start_request = ToolRequest {
        tool_name: "calibration_manager".to_string(),
        params: json!({
            "action": "start_calibration",
            "device_id": device_id
        }),
    };
    let start_response = server.call_tool(start_request).await?;
    let start_result = start_response.result;
    
    if let Some(session_id) = start_result["session_id"].as_str() {
        eprintln!("   Calibration started with session: {}", session_id);
        
        // Monitor for a short time
        for i in 0..10 {
            sleep(Duration::from_secs(1)).await;
            
            let status_request = ToolRequest {
                tool_name: "calibration_manager".to_string(),
                params: json!({
                    "action": "get_status",
                    "session_id": session_id
                }),
            };
            let status_response = server.call_tool(status_request).await?;
            let status = status_response.result;
            
            eprintln!("   [{}s] Status: {} (elapsed: {}s)", 
                i + 1, 
                status["status"].as_str().unwrap_or("unknown"),
                status["elapsed_seconds"].as_u64().unwrap_or(0)
            );
            
            if status["status"].as_str() == Some("complete") {
                eprintln!("\n   ✓ Calibration completed!");
                break;
            }
        }
    }
    
    eprintln!("\n=== DEBUG TEST COMPLETE ===\n");
    
    Ok(())
}