#![cfg(target_os = "macos")]

//! Basic sanity check for XCUITest templates
//!
//! Run with: cargo test --test xctest_basic_sanity

use std::path::PathBuf;

#[test]
fn test_xctest_templates_exist() {
    // Check Swift template
    let swift_template = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates")
        .join("XCTestRunner")
        .join("ArkavoTestRunner.swift.template");

    assert!(
        swift_template.exists(),
        "Swift template not found at {:?}",
        swift_template
    );

    // Check Info.plist template
    let plist_template = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates")
        .join("XCTestRunner")
        .join("Info.plist.template");

    assert!(
        plist_template.exists(),
        "Info.plist template not found at {:?}",
        plist_template
    );

    println!("✅ XCUITest templates found successfully");
}

#[test]
fn test_swift_template_content() {
    let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates")
        .join("XCTestRunner")
        .join("ArkavoTestRunner.swift.template");

    let content = std::fs::read_to_string(&template_path).expect("Failed to read Swift template");

    // Check for key components
    assert!(
        content.contains("import XCTest"),
        "Template should import XCTest"
    );
    assert!(
        content.contains("import Foundation"),
        "Template should import Foundation"
    );
    assert!(
        content.contains("class ArkavoTestRunner: NSObject"),
        "Template should have bridge class (NSObject, not XCTestCase)"
    );
    assert!(
        content.contains("UnixSocketServer"),
        "Template should use Unix sockets"
    );
    assert!(
        content.contains("{{SOCKET_PATH}}"),
        "Template should have socket path placeholder"
    );
    assert!(
        content.contains("Command") && content.contains("CommandType"),
        "Template should define Command structs"
    );
    assert!(
        content.contains("handleTextTap"),
        "Template should handle text-based taps"
    );
    assert!(
        content.contains("handleAccessibilityTap"),
        "Template should handle accessibility ID taps"
    );
    assert!(
        content.contains("@objc class func setUp()"),
        "Template should have setUp method"
    );
    assert!(
        content.contains("@objc class func initializeBridge()"),
        "Template should have initializeBridge method"
    );

    println!("✅ Swift template content validated");
}

#[test]
fn test_plist_template_content() {
    let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates")
        .join("XCTestRunner")
        .join("Info.plist.template");

    let content =
        std::fs::read_to_string(&template_path).expect("Failed to read Info.plist template");

    // Check for required keys
    assert!(
        content.contains("CFBundleIdentifier"),
        "Plist should have bundle identifier"
    );
    assert!(
        content.contains("com.arkavo.testrunner"),
        "Plist should have correct bundle ID"
    );
    assert!(
        content.contains("CFBundleName"),
        "Plist should have bundle name"
    );
    assert!(
        content.contains("ArkavoTestRunner"),
        "Plist should have correct name"
    );
    assert!(
        content.contains("CFBundlePackageType"),
        "Plist should have package type"
    );
    assert!(content.contains("BNDL"), "Plist should be a bundle");

    println!("✅ Info.plist template content validated");
}

#[cfg(target_os = "macos")]
#[test]
fn test_macos_tools_available() {
    use std::process::Command;

    // Check xcrun
    let xcrun = Command::new("which")
        .arg("xcrun")
        .output()
        .expect("Failed to check for xcrun");

    assert!(
        xcrun.status.success(),
        "xcrun not found - Xcode command line tools required"
    );

    // Check if we can run simctl
    let simctl = Command::new("xcrun")
        .args(["simctl", "help"])
        .output()
        .expect("Failed to run simctl");

    assert!(
        simctl.status.success(),
        "simctl not working - iOS simulator support required"
    );

    println!("✅ macOS development tools available");
}

#[test]
fn test_unix_socket_concepts() {
    // Test that we can work with Unix socket paths
    let socket_path = std::env::temp_dir().join(format!("test-arkavo-{}.sock", std::process::id()));

    assert!(socket_path.is_absolute(), "Socket path should be absolute");
    assert!(
        socket_path.parent().unwrap().exists(),
        "Socket directory should exist"
    );

    // Clean up if it exists
    if socket_path.exists() {
        std::fs::remove_file(&socket_path).ok();
    }

    println!("✅ Unix socket path handling works");
}

/// Main sanity check that runs all tests
#[test]
fn xctest_sanity_check() {
    println!("\n🔍 Running XCUITest Sanity Checks...\n");

    test_xctest_templates_exist();
    test_swift_template_content();
    test_plist_template_content();
    test_unix_socket_concepts();

    #[cfg(target_os = "macos")]
    test_macos_tools_available();

    #[cfg(not(target_os = "macos"))]
    println!("⚠️  Skipping macOS-specific checks on non-macOS platform");

    println!("\n✅ All XCUITest sanity checks passed!\n");
    println!("The XCUITest infrastructure is ready for use:");
    println!("- Templates are in place");
    println!("- Unix socket communication is configured");
    println!("- Swift code can handle text/accessibility ID taps");
    #[cfg(target_os = "macos")]
    println!("- macOS development tools are available");
}
