//! Integration tests for XCUITest functionality
//!
//! These tests verify that the XCUITest runner can be compiled, deployed,
//! and used to interact with iOS simulators.

use arkavo_test::{Result, TestError};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Check if we're running on macOS
fn is_macos() -> bool {
    cfg!(target_os = "macos")
}

/// Check if iOS simulators are available
async fn has_ios_simulators() -> bool {
    if !is_macos() {
        return false;
    }

    // Try to list simulators
    let output = tokio::process::Command::new("xcrun")
        .args(&["simctl", "list", "devices", "--json"])
        .output()
        .await;

    match output {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

#[tokio::test]
async fn test_xctest_compiler_creates_bundle() {
    if !is_macos() {
        eprintln!("Skipping test - not running on macOS");
        return;
    }

    // Create compiler
    let compiler = XCTestCompiler::new().expect("Failed to create compiler");

    // Compile bundle
    let bundle_path = compiler
        .get_xctest_bundle()
        .expect("Failed to compile XCTest bundle");

    // Verify bundle exists
    assert!(
        bundle_path.exists(),
        "XCTest bundle should exist at {:?}",
        bundle_path
    );

    // Verify bundle structure
    let info_plist = bundle_path.join("Info.plist");
    assert!(info_plist.exists(), "Bundle should contain Info.plist");

    let binary = bundle_path.join("ArkavoTestRunner");
    assert!(binary.exists(), "Bundle should contain executable");
}

#[tokio::test]
async fn test_unix_socket_bridge_lifecycle() {
    if !is_macos() {
        eprintln!("Skipping test - not running on macOS");
        return;
    }

    // Create bridge
    let mut bridge = XCTestUnixBridge::new();
    let socket_path = bridge.socket_path().to_path_buf();

    // Start bridge
    bridge
        .start()
        .await
        .expect("Failed to start Unix socket bridge");

    // Verify socket file was created
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(socket_path.exists(), "Socket file should exist");

    // Drop bridge to test cleanup
    drop(bridge);

    // Verify socket file was cleaned up
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!socket_path.exists(), "Socket file should be cleaned up");
}

#[tokio::test]
async fn test_device_manager_finds_simulators() {
    if !is_macos() || !has_ios_simulators().await {
        eprintln!("Skipping test - iOS simulators not available");
        return;
    }

    let device_manager = Arc::new(DeviceManager::new());

    // Refresh devices
    device_manager
        .refresh_devices()
        .expect("Failed to refresh devices");

    // Get all devices
    let devices = device_manager.get_all_devices();
    assert!(!devices.is_empty(), "Should find at least one iOS device");

    // Check for iPhone devices
    let iphone_devices: Vec<_> = devices
        .iter()
        .filter(|d| d.device_type.contains("iPhone"))
        .collect();

    assert!(
        !iphone_devices.is_empty(),
        "Should find at least one iPhone simulator"
    );

    // Print found devices for debugging
    for device in &iphone_devices {
        eprintln!(
            "Found device: {} ({}) - {}",
            device.name, device.id, device.state
        );
    }
}

#[tokio::test]
async fn test_simulator_boot_and_xctest_integration() {
    if !is_macos() || !has_ios_simulators().await {
        eprintln!("Skipping test - iOS simulators not available");
        return;
    }

    let device_manager = Arc::new(DeviceManager::new());
    let simulator_manager = SimulatorManager::new();

    // Refresh and find a device
    device_manager
        .refresh_devices()
        .expect("Failed to refresh devices");

    // Find an iPhone simulator
    let iphone = device_manager
        .get_all_devices()
        .into_iter()
        .find(|d| d.device_type.contains("iPhone") && !d.device_type.contains("iPad"))
        .expect("No iPhone simulator found");

    eprintln!("Using device: {} ({})", iphone.name, iphone.id);

    // Boot simulator if needed
    if iphone.state != "Booted" {
        eprintln!("Booting simulator...");
        simulator_manager
            .boot_simulator(&iphone.id)
            .await
            .expect("Failed to boot simulator");

        // Wait for boot
        tokio::time::sleep(Duration::from_secs(10)).await;
    }

    // Set as active device
    device_manager
        .set_active_device(&iphone.id)
        .expect("Failed to set active device");

    // Create XCTest compiler and bridge
    let compiler = XCTestCompiler::new().expect("Failed to create compiler");
    let bundle_path = compiler
        .get_xctest_bundle()
        .expect("Failed to compile bundle");

    eprintln!("Installing XCTest bundle to simulator...");

    // Install bundle
    compiler
        .install_to_simulator(&iphone.id, &bundle_path)
        .expect("Failed to install bundle");

    // Start Unix socket bridge
    let mut bridge = XCTestUnixBridge::new();
    bridge.start().await.expect("Failed to start bridge");

    eprintln!("XCTest infrastructure ready");

    // Note: Actually running the test bundle requires more setup
    // and would launch XCTest on the simulator. This test verifies
    // the infrastructure can be set up correctly.
}

#[tokio::test]
async fn test_tap_command_serialization() {
    // Test coordinate tap
    let coord_tap = XCTestUnixBridge::create_coordinate_tap(100.0, 200.0);
    assert_eq!(coord_tap.target_type, TargetType::Coordinate);
    assert_eq!(coord_tap.x, Some(100.0));
    assert_eq!(coord_tap.y, Some(200.0));

    // Verify JSON serialization
    let json = serde_json::to_string(&coord_tap).expect("Failed to serialize");
    assert!(json.contains("\"targetType\":\"coordinate\""));
    assert!(json.contains("\"x\":100.0"));
    assert!(json.contains("\"y\":200.0"));

    // Test text tap
    let text_tap = XCTestUnixBridge::create_text_tap("Login".to_string(), Some(5.0));
    assert_eq!(text_tap.target_type, TargetType::Text);
    assert_eq!(text_tap.text, Some("Login".to_string()));
    assert_eq!(text_tap.timeout, Some(5.0));

    // Test accessibility tap
    let acc_tap = XCTestUnixBridge::create_accessibility_tap("login_button".to_string(), None);
    assert_eq!(acc_tap.target_type, TargetType::AccessibilityId);
    assert_eq!(acc_tap.accessibility_id, Some("login_button".to_string()));
}

#[tokio::test]
async fn test_full_tap_flow_mock() {
    if !is_macos() {
        eprintln!("Skipping test - not running on macOS");
        return;
    }

    // This test demonstrates the full flow without actually running XCTest
    // In a real scenario, you'd have the XCTest runner connected

    let mut bridge = XCTestUnixBridge::new();
    bridge.start().await.expect("Failed to start bridge");

    // Create a tap command
    let tap_cmd = XCTestUnixBridge::create_text_tap("Settings".to_string(), Some(10.0));

    // In a real test, we'd send this command and wait for response
    // For now, we just verify the command structure
    assert!(!tap_cmd.id.is_empty(), "Command should have ID");
    assert_eq!(tap_cmd.text, Some("Settings".to_string()));

    // Test would normally do:
    // let response = bridge.send_tap_command(tap_cmd).await?;
    // assert!(response.success);
}

/// Helper test to verify XCode command line tools are installed
#[tokio::test]
async fn test_xcode_tools_available() {
    if !is_macos() {
        eprintln!("Skipping test - not running on macOS");
        return;
    }

    // Check xcrun
    let xcrun_check = tokio::process::Command::new("xcrun")
        .arg("--version")
        .output()
        .await;

    assert!(xcrun_check.is_ok(), "xcrun should be available");

    // Check xcodebuild
    let xcodebuild_check = tokio::process::Command::new("xcodebuild")
        .arg("-version")
        .output()
        .await;

    if xcodebuild_check.is_err() {
        eprintln!("WARNING: xcodebuild not available - some functionality may be limited");
    }

    // Check simctl
    let simctl_check = tokio::process::Command::new("xcrun")
        .args(&["simctl", "help"])
        .output()
        .await;

    assert!(simctl_check.is_ok(), "simctl should be available");
}
