#![cfg(target_os = "macos")]

//! Quick sanity check for XCUITest integration
//!
//! Run with: cargo test --test xctest_sanity

use std::path::PathBuf;

#[test]
fn test_swift_template_exists() {
    let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates")
        .join("XCTestRunner")
        .join("ArkavoTestRunner.swift.template");

    assert!(
        template_path.exists(),
        "Swift template should exist at {:?}",
        template_path
    );

    // Verify template contains expected markers
    let content = std::fs::read_to_string(&template_path).expect("Should be able to read template");

    assert!(
        content.contains("{{SOCKET_PATH}}"),
        "Template should contain SOCKET_PATH placeholder"
    );

    assert!(
        content.contains("class ArkavoTestRunner: NSObject"),
        "Template should contain bridge class (NSObject, not XCTestCase)"
    );

    assert!(
        content.contains("UnixSocketServer"),
        "Template should contain Unix socket server"
    );
}

#[test]
fn test_info_plist_template_exists() {
    let plist_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates")
        .join("XCTestRunner")
        .join("Info.plist.template");

    assert!(
        plist_path.exists(),
        "Info.plist template should exist at {:?}",
        plist_path
    );

    // Verify plist contains expected keys
    let content = std::fs::read_to_string(&plist_path).expect("Should be able to read plist");

    assert!(
        content.contains("com.arkavo.testrunner"),
        "Plist should contain bundle identifier"
    );

    assert!(
        content.contains("ArkavoTestRunner"),
        "Plist should contain bundle name"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_dependencies() {
    use std::process::Command;

    // Check if xcrun is available
    let xcrun_output = Command::new("which")
        .arg("xcrun")
        .output()
        .expect("Failed to run which command");

    assert!(
        xcrun_output.status.success(),
        "xcrun should be available on macOS"
    );

    // Check if we can query simulators
    let simctl_output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "-j"])
        .output()
        .expect("Failed to run simctl");

    assert!(
        simctl_output.status.success(),
        "Should be able to list iOS simulators"
    );

    // Parse JSON to verify format
    let json_str = String::from_utf8_lossy(&simctl_output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&json_str).expect("simctl output should be valid JSON");

    assert!(
        json.get("devices").is_some(),
        "simctl output should contain devices"
    );
}

#[test]
fn test_unix_socket_path_generation() {
    use arkavo_test::mcp::xctest_unix_bridge::XCTestUnixBridge;

    let bridge = XCTestUnixBridge::new();
    let socket_path = bridge.socket_path();

    // Verify path is in temp directory
    assert!(
        socket_path.starts_with(std::env::temp_dir()),
        "Socket path should be in temp directory"
    );

    // Verify path contains expected prefix
    assert!(
        socket_path.to_string_lossy().contains("arkavo-xctest"),
        "Socket path should contain arkavo-xctest prefix"
    );

    // Verify path has .sock extension
    assert!(
        socket_path.to_string_lossy().ends_with(".sock"),
        "Socket path should end with .sock"
    );
}

#[test]
fn test_command_structure_compatibility() {
    use arkavo_test::mcp::xctest_unix_bridge::{
        Command, CommandParameters, CommandType, TargetType,
    };

    // Test that our Rust structures can be serialized to match Swift expectations
    let tap_cmd = Command {
        id: "test-123".to_string(),
        command_type: CommandType::Tap,
        parameters: CommandParameters {
            target_type: Some(TargetType::Text),
            x: None,
            y: None,
            text: Some("Login".to_string()),
            accessibility_id: None,
            timeout: Some(5.0),
            x1: None,
            y1: None,
            x2: None,
            y2: None,
            duration: None,
            text_to_type: None,
            clear_first: None,
            direction: None,
            distance: None,
            press_duration: None,
        },
    };

    let json = serde_json::to_string(&tap_cmd).expect("Should serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("Should parse");

    // Verify JSON structure matches Swift expectations
    assert_eq!(parsed["id"], "test-123");
    assert_eq!(parsed["type"], "tap");
    assert_eq!(parsed["parameters"]["targetType"], "text");
    assert_eq!(parsed["parameters"]["text"], "Login");
    assert_eq!(parsed["parameters"]["timeout"], 5.0);

    // Verify null fields are properly handled
    assert!(parsed["parameters"]["x"].is_null());
    assert!(parsed["parameters"]["y"].is_null());
    assert!(parsed["parameters"]["accessibilityId"].is_null());
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_xctest_compiler_socket_path() {
    use arkavo_test::mcp::xctest_compiler::XCTestCompiler;

    let compiler = XCTestCompiler::new().expect("Should create compiler");
    let socket_path = compiler.socket_path();

    // Verify socket path is reasonable
    assert!(socket_path.is_absolute(), "Socket path should be absolute");
    assert!(
        socket_path.parent().is_some(),
        "Socket path should have parent directory"
    );

    // Verify parent directory exists (temp dir)
    assert!(
        socket_path.parent().unwrap().exists(),
        "Socket path parent directory should exist"
    );
}

/// Run all sanity checks
#[test]
fn xctest_sanity_check_all() {
    println!("Running XCUITest sanity checks...");

    // Templates exist
    test_swift_template_exists();
    test_info_plist_template_exists();

    // Data structures are compatible
    test_unix_socket_path_generation();
    test_command_structure_compatibility();

    // Platform-specific checks
    #[cfg(target_os = "macos")]
    {
        test_macos_dependencies();
        println!("✅ All macOS-specific checks passed");
    }

    #[cfg(not(target_os = "macos"))]
    {
        println!("⚠️  Skipping macOS-specific checks on non-macOS platform");
    }

    println!("✅ All XCUITest sanity checks passed!");
}
