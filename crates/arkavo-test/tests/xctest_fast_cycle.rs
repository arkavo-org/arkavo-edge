//! Fast test cycle for XCTest bridge functionality
//! 
//! This test demonstrates how to quickly verify XCTest functionality
//! without going through the full MCP server setup and AI agent flow.

use arkavo_test::mcp::{
    device_xctest_status::DeviceXCTestStatusManager,
    device_manager::DeviceManager,
    xctest_verifier::XCTestVerifier,
};
use std::sync::Arc;

#[tokio::test]
async fn test_fast_xctest_verification_cycle() {
    println!("\n=== Fast XCTest Verification Cycle ===\n");
    
    // Step 1: Quick check if any XCTest functionality is available
    println!("1. Quick system check...");
    match XCTestVerifier::quick_verify().await {
        Ok(is_functional) => {
            println!("   XCTest functional: {}", is_functional);
            if !is_functional {
                println!("   Note: This is expected in CI or without simulators");
            }
        }
        Err(e) => {
            println!("   Error: {} (expected in CI)", e);
        }
    }
    
    // Step 2: Check device manager
    println!("\n2. Checking device manager...");
    let device_manager = Arc::new(DeviceManager::new());
    match device_manager.refresh_devices() {
        Ok(devices) => {
            println!("   Found {} devices", devices.len());
            for device in &devices {
                println!("   - {} ({}) - {:?}", device.name, device.id, device.state);
            }
        }
        Err(e) => {
            println!("   No devices found: {} (expected without Xcode)", e);
            return;
        }
    }
    
    // Step 3: Check XCTest status for all devices
    println!("\n3. Checking XCTest status for all devices...");
    match DeviceXCTestStatusManager::get_all_devices_with_status(device_manager.clone()).await {
        Ok(devices_with_status) => {
            for device_status in devices_with_status {
                println!("\n   Device: {} ({})", device_status.device.name, device_status.device.id);
                println!("   State: {:?}", device_status.device.state);
                
                if let Some(xctest_status) = &device_status.xctest_status {
                    println!("   XCTest Functional: {}", xctest_status.is_functional);
                    println!("   Bundle Installed: {}", xctest_status.bundle_installed);
                    println!("   Bridge Connectable: {}", xctest_status.bridge_connectable);
                    
                    if let Some(response_time) = xctest_status.swift_response_time {
                        println!("   Response Time: {:?}", response_time);
                    }
                    
                    if let Some(error) = &xctest_status.error_details {
                        println!("   Error: {} (stage: {})", error.message, error.stage);
                        println!("   Can Retry: {}", error.can_retry);
                    }
                } else {
                    println!("   XCTest status: Not checked (device not booted)");
                }
            }
        }
        Err(e) => {
            println!("   Failed to get device status: {}", e);
        }
    }
    
    // Step 4: Find best device for XCTest
    println!("\n4. Finding best device for XCTest operations...");
    match DeviceXCTestStatusManager::find_best_xctest_device(device_manager.clone()).await {
        Ok(Some(best_device)) => {
            println!("   Best device: {} ({})", best_device.device.name, best_device.device.id);
            if let Some(status) = &best_device.xctest_status {
                if status.is_functional {
                    println!("   ✅ Ready for XCTest operations!");
                } else if status.bundle_installed {
                    println!("   ⚠️  XCTest installed but not functional");
                } else {
                    println!("   ❌ XCTest not installed");
                }
            }
        }
        Ok(None) => {
            println!("   No devices available");
        }
        Err(e) => {
            println!("   Error finding best device: {}", e);
        }
    }
    
    println!("\n=== End of Fast Test Cycle ===\n");
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore] // Run with: cargo test test_xctest_setup_and_verify -- --ignored
async fn test_xctest_setup_and_verify() {
    use arkavo_test::mcp::xctest_setup_tool::XCTestSetupKit;
    use arkavo_test::mcp::server::Tool;
    use serde_json::json;
    
    println!("\n=== XCTest Setup and Verification Test ===\n");
    
    let device_manager = Arc::new(DeviceManager::new());
    
    // Find a booted device
    let devices = device_manager.refresh_devices().expect("Failed to get devices");
    let booted_device = devices.iter()
        .find(|d| d.state == arkavo_test::mcp::device_manager::DeviceState::Booted);
        
    if let Some(device) = booted_device {
        println!("Testing on device: {} ({})", device.name, device.id);
        
        // Step 1: Check initial status
        println!("\n1. Initial XCTest status:");
        let initial_status = XCTestVerifier::verify_device(&device.id).await
            .expect("Failed to verify device");
        println!("   Functional: {}", initial_status.is_functional);
        println!("   Bundle Installed: {}", initial_status.bundle_installed);
        
        // Step 2: Setup XCTest if needed
        if !initial_status.is_functional {
            println!("\n2. Setting up XCTest...");
            let setup_kit = XCTestSetupKit::new(device_manager.clone());
            let params = json!({
                "device_id": device.id,
                "force_reinstall": false
            });
            
            match setup_kit.execute(params).await {
                Ok(result) => {
                    if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                        println!("   ✅ XCTest setup successful!");
                        if let Some(device_status) = result.get("device_status") {
                            println!("   Device status: {}", serde_json::to_string_pretty(device_status).unwrap());
                        }
                    } else {
                        println!("   ❌ XCTest setup failed: {}", serde_json::to_string_pretty(&result).unwrap());
                    }
                }
                Err(e) => {
                    println!("   ❌ Setup error: {}", e);
                }
            }
        }
        
        // Step 3: Verify final status
        println!("\n3. Final XCTest status:");
        let final_status = XCTestVerifier::verify_device(&device.id).await
            .expect("Failed to verify device");
        println!("   Functional: {}", final_status.is_functional);
        println!("   Bundle Installed: {}", final_status.bundle_installed);
        if let Some(response_time) = final_status.swift_response_time {
            println!("   Response Time: {:?}", response_time);
        }
    } else {
        println!("No booted simulator found. Please boot a simulator first.");
    }
    
    println!("\n=== End of Setup and Verification Test ===\n");
}

/// Benchmark test to measure XCTest verification performance
#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore] // Run with: cargo test test_xctest_verification_performance -- --ignored
async fn test_xctest_verification_performance() {
    use std::time::Instant;
    
    println!("\n=== XCTest Verification Performance Test ===\n");
    
    // Test quick verify performance
    let start = Instant::now();
    let _ = XCTestVerifier::quick_verify().await;
    let quick_duration = start.elapsed();
    println!("Quick verify took: {:?}", quick_duration);
    assert!(quick_duration.as_secs() < 5, "Quick verify should complete within 5 seconds");
    
    // Test full device status check performance
    let device_manager = Arc::new(DeviceManager::new());
    let start = Instant::now();
    let _ = DeviceXCTestStatusManager::get_all_devices_with_status(device_manager).await;
    let full_duration = start.elapsed();
    println!("Full device status check took: {:?}", full_duration);
    assert!(full_duration.as_secs() < 10, "Full status check should complete within 10 seconds");
    
    println!("\n=== End of Performance Test ===\n");
}