#![cfg(target_os = "macos")]

use arkavo_test::mcp::calibration::server::{
    CalibrationRequest, CalibrationResponse, CalibrationServer,
};
use std::path::PathBuf;

#[tokio::test]
async fn test_calibration_workflow() {
    println!("\n=== Calibration System Demonstration ===\n");

    // Step 1: Initialize calibration server
    println!("1. Initializing calibration server...");
    let storage_path = PathBuf::from("/tmp/arkavo_calibration_demo");
    let server = CalibrationServer::new(storage_path).expect("Failed to create calibration server");

    // Step 2: List currently calibrated devices (should be empty initially)
    println!("\n2. Listing calibrated devices...");
    let devices = server.data_store.list_calibrated_devices();
    println!("   Found {} calibrated devices", devices.len());
    for device in &devices {
        println!("   - {} (valid: {})", device.device_id, device.is_valid);
    }

    // Step 3: Get a test device ID (simulated)
    let device_id = "test-device-001".to_string();
    println!("\n3. Using test device: {}", device_id);

    // Step 4: Start calibration
    println!("\n4. Starting calibration...");
    let start_request = CalibrationRequest::StartCalibration {
        device_id: device_id.clone(),
        reference_bundle_id: Some("com.arkavo.reference".to_string()),
    };

    let session_id = match server.handle_request(start_request).await {
        CalibrationResponse::SessionStarted { session_id } => {
            println!("   ✓ Calibration started successfully");
            println!("   Session ID: {}", session_id);
            session_id
        }
        CalibrationResponse::Error { message } => {
            println!("   ✗ Failed to start calibration: {}", message);
            return;
        }
        _ => {
            println!("   ✗ Unexpected response");
            return;
        }
    };

    // Step 5: Check calibration status iteratively
    println!("\n5. Monitoring calibration progress...");
    for i in 1..=10 {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let status_request = CalibrationRequest::GetStatus {
            session_id: session_id.clone(),
        };

        if let CalibrationResponse::Status(status) = server.handle_request(status_request).await {
            println!("   Check {}/10: {}", i, status.status);

            if status.status.contains("complete") {
                println!("   ✓ Calibration completed!");
                break;
            } else if status.status.contains("failed") {
                println!("   ✗ Calibration failed!");
                break;
            }
        }
    }

    // Step 6: Retrieve calibration data
    println!("\n6. Retrieving calibration data...");
    let get_request = CalibrationRequest::GetCalibration {
        device_id: device_id.clone(),
    };

    match server.handle_request(get_request).await {
        CalibrationResponse::CalibrationData { data } => {
            println!("   ✓ Calibration data retrieved successfully");
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(config) = parsed.get("config") {
                    println!("   Device type: {}", config["device_type"]);
                    println!(
                        "   Screen size: {}x{}",
                        config["screen_size"]["width"], config["screen_size"]["height"]
                    );
                }
                if let Some(result) = parsed.get("result") {
                    println!("   Success: {}", result["success"]);
                    println!(
                        "   Validation accuracy: {}%",
                        result["validation_report"]["accuracy_percentage"]
                    );
                }
            }
        }
        CalibrationResponse::Error { message } => {
            println!("   ✗ Failed to retrieve calibration: {}", message);
        }
        _ => {}
    }

    // Step 7: Enable auto-monitoring
    println!("\n7. Enabling auto-monitoring...");
    let monitor_request = CalibrationRequest::EnableAutoMonitoring { enabled: true };

    if let CalibrationResponse::Success = server.handle_request(monitor_request).await {
        println!("   ✓ Auto-monitoring enabled");
        println!("   System will automatically recalibrate devices when needed");
    }

    // Step 8: Demonstrate calibration usage
    println!("\n8. How calibration data is used:");
    println!("   - Coordinate mapping adjusts tap locations");
    println!("   - Interaction adjustments handle element-specific quirks");
    println!("   - Edge cases provide fallback strategies");
    println!("   - Validation reports ensure accuracy");

    println!("\n=== Calibration Demo Complete ===\n");
}

#[tokio::test]
async fn test_calibration_with_real_simulator() {
    println!("\n=== Real Simulator Calibration Demo ===\n");

    // Get list of available simulators
    let output = std::process::Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted", "-j"])
        .output()
        .expect("Failed to list simulators");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Failed to parse simulator list");

    // Extract first booted device
    let mut device_id = None;
    if let Some(devices) = json["devices"].as_object() {
        for (_runtime, device_list) in devices {
            if let Some(array) = device_list.as_array() {
                if let Some(first_device) = array.first() {
                    device_id = first_device["udid"].as_str().map(|s| s.to_string());
                    if device_id.is_some() {
                        break;
                    }
                }
            }
        }
    }

    if let Some(device_id) = device_id {
        println!("Found booted simulator: {}", device_id);

        // Initialize server and run calibration
        let storage_path = PathBuf::from("/tmp/arkavo_calibration_real");
        let server =
            CalibrationServer::new(storage_path).expect("Failed to create calibration server");

        // Start calibration
        let start_request = CalibrationRequest::StartCalibration {
            device_id: device_id.clone(),
            reference_bundle_id: None,
        };

        match server.handle_request(start_request).await {
            CalibrationResponse::SessionStarted { session_id } => {
                println!("Calibration started with session: {}", session_id);

                // The actual calibration would run here
                // For demo purposes, we just show it started
            }
            CalibrationResponse::Error { message } => {
                println!("Failed to start calibration: {}", message);
            }
            _ => {}
        }
    } else {
        println!("No booted simulators found. Boot a simulator first with:");
        println!("  xcrun simctl boot <device-id>");
    }

    println!("\n=== Demo Complete ===\n");
}
