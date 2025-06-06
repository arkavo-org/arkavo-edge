use arkavo_test::mcp::xctest_verifier::{XCTestStatus, XCTestVerifier};
use std::time::Duration;

#[tokio::test]
async fn test_xctest_quick_verify() {
    // This test runs quickly without needing full MCP server setup
    let result = XCTestVerifier::quick_verify().await;

    match result {
        Ok(is_functional) => {
            println!("XCTest bridge functional status: {}", is_functional);
            if !is_functional {
                println!("XCTest bridge is not functional - this is expected in CI environments");
            }
        }
        Err(e) => {
            println!(
                "XCTest verification error: {} - this is expected in CI environments",
                e
            );
        }
    }
}

#[tokio::test]
#[ignore] // Run with: cargo test test_xctest_device_verification -- --ignored
async fn test_xctest_device_verification() {
    // This test requires a booted simulator
    use std::process::Command;

    // List booted devices
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted", "-j"])
        .output()
        .expect("Failed to list devices");

    let json_str = String::from_utf8_lossy(&output.stdout);
    let devices: serde_json::Value = serde_json::from_str(&json_str).expect("Failed to parse JSON");

    // Find first booted device
    let mut device_id = None;
    if let Some(devices_map) = devices.get("devices").and_then(|d| d.as_object()) {
        for (_runtime, device_list) in devices_map {
            if let Some(devices_array) = device_list.as_array() {
                for device in devices_array {
                    if let Some(state) = device.get("state").and_then(|s| s.as_str()) {
                        if state == "Booted" {
                            device_id = device
                                .get("udid")
                                .and_then(|u| u.as_str())
                                .map(|s| s.to_string());
                            break;
                        }
                    }
                }
            }
            if device_id.is_some() {
                break;
            }
        }
    }

    let device_id = device_id.expect("No booted simulator found. Please boot a simulator first.");

    println!("Testing XCTest verification on device: {}", device_id);

    let status = XCTestVerifier::verify_device(&device_id)
        .await
        .expect("Verification failed");

    println!("XCTest Status for device {}:", device_id);
    println!("  Functional: {}", status.is_functional);
    println!("  Bundle Installed: {}", status.bundle_installed);
    println!("  Bridge Connectable: {}", status.bridge_connectable);
    if let Some(response_time) = status.swift_response_time {
        println!("  Response Time: {:?}", response_time);
    }
    if let Some(error) = &status.error_details {
        println!("  Error: {} at stage {}", error.message, error.stage);
        println!("  Can Retry: {}", error.can_retry);
    }
}

#[test]
fn test_xctest_status_structure() {
    // Test that our status structure is properly designed
    let status = XCTestStatus {
        device_id: "test-device".to_string(),
        is_functional: false,
        bundle_installed: true,
        bridge_connectable: false,
        swift_response_time: Some(Duration::from_millis(500)),
        error_details: Some(arkavo_test::mcp::xctest_verifier::XCTestError {
            stage: "bridge_test".to_string(),
            message: "Connection timeout".to_string(),
            can_retry: true,
        }),
    };

    // Serialize to JSON to verify it works well for MCP responses
    let json = serde_json::to_string_pretty(&status).unwrap();
    println!("XCTest status JSON:\n{}", json);

    // Verify we can deserialize it back
    let parsed: XCTestStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.device_id, "test-device");
    assert!(!parsed.is_functional);
    assert!(parsed.bundle_installed);
    assert!(!parsed.bridge_connectable);
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_xctest_bridge_lifecycle() {
    use arkavo_test::mcp::xctest_unix_bridge::XCTestUnixBridge;

    // Test creating and starting a bridge
    let socket_path =
        std::env::temp_dir().join(format!("test-xctest-bridge-{}.sock", std::process::id()));
    let mut bridge = XCTestUnixBridge::with_socket_path(socket_path.clone());

    // Verify socket path
    assert_eq!(bridge.socket_path(), socket_path);
    assert!(!bridge.is_connected());

    // Start the bridge
    bridge.start().await.expect("Failed to start bridge");

    // Verify socket file was created
    assert!(socket_path.exists(), "Socket file should exist after start");

    // Bridge should still not be connected (no client yet)
    assert!(!bridge.is_connected());

    // Clean up happens automatically in Drop
}

/// Fast test to ensure XCTest components compile and basic structures work
#[test]
fn test_xctest_components_compile() {
    use arkavo_test::mcp::{
        xctest_compiler::XCTestCompiler,
        xctest_unix_bridge::{Command, CommandType},
    };

    // Test that we can create instances without runtime errors
    let _ = XCTestCompiler::new(); // May fail if Xcode not installed, but that's OK

    // Test command creation
    let tap_cmd = Command {
        id: "test".to_string(),
        command_type: CommandType::Tap,
        parameters: Default::default(),
    };

    assert_eq!(tap_cmd.command_type, CommandType::Tap);
}
