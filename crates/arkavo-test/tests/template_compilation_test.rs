use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn test_xctest_template_compiles() {
    // Skip test if not on macOS
    if !cfg!(target_os = "macos") {
        println!("Skipping XCTest template compilation test on non-macOS platform");
        return;
    }
    
    // Check if Xcode is available
    let xcode_check = Command::new("xcrun")
        .args(["--version"])
        .output();
        
    if xcode_check.is_err() || !xcode_check.unwrap().status.success() {
        println!("Skipping XCTest template compilation test - Xcode not available");
        return;
    }
    
    // Create a temporary directory for compilation
    let temp_dir = std::env::temp_dir().join("arkavo-xctest-template-validation");
    fs::create_dir_all(&temp_dir).expect("Failed to create temp directory");
    
    // Read the template
    let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates/XCTestRunner/ArkavoTestRunner.swift.template");
    let template_content = fs::read_to_string(&template_path)
        .expect("Failed to read Swift template");
    
    // Replace template variable with a test socket path
    let swift_content = template_content.replace("{{SOCKET_PATH}}", "/tmp/test.sock");
    
    // Write to a temporary Swift file
    let swift_file = temp_dir.join("ArkavoTestRunner.swift");
    fs::write(&swift_file, swift_content).expect("Failed to write Swift file");
    
    // Get the SDK path dynamically
    let sdk_output = Command::new("xcrun")
        .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
        .output()
        .expect("Failed to get SDK path");
        
    let sdk_path = String::from_utf8_lossy(&sdk_output.stdout).trim().to_string();
    
    // Try to compile the Swift file
    let compile_output = Command::new("xcrun")
        .args([
            "swiftc",
            "-sdk", &sdk_path,
            "-target", "x86_64-apple-ios15.0-simulator",
            "-emit-library",
            "-emit-module", 
            "-module-name", "ArkavoTestRunner",
            "-F", "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/Library/Frameworks",
            "-framework", "XCTest",
            "-o", temp_dir.join("ArkavoTestRunner.dylib").to_str().unwrap(),
            swift_file.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run Swift compiler");
    
    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);
    
    // Check compilation result
    if !compile_output.status.success() {
        let stderr = String::from_utf8_lossy(&compile_output.stderr);
        panic!(
            "Swift template failed to compile:\n{}\n\nThis means the template has syntax errors that will fail at runtime!",
            stderr
        );
    }
    
    println!("✓ XCTest template compiled successfully");
}

#[test]
fn test_template_has_required_components() {
    let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("templates/XCTestRunner/ArkavoTestRunner.swift.template");
    let content = fs::read_to_string(&template_path)
        .expect("Failed to read Swift template");
    
    // Check for required components
    assert!(content.contains("import XCTest"), "Template must import XCTest");
    assert!(content.contains("class ArkavoTestRunner: XCTestCase"), "Template must have test class");
    assert!(content.contains("func testRunCommands()"), "Template must have test method");
    assert!(content.contains("UnixSocketServer"), "Template must have socket server");
    assert!(content.contains("{{SOCKET_PATH}}"), "Template must have socket path placeholder");
    
    // Check for common mistakes
    assert_eq!(
        content.matches("func testRunCommands()").count(), 
        1, 
        "Template should have exactly one testRunCommands method"
    );
    
    // Ensure we're not using unavailable macros
    assert!(!content.contains("XCTFail("), "Template should not use XCTFail macro");
    assert!(!content.contains("XCTAssertTrue("), "Template should not use XCTAssertTrue macro");
    
    println!("✓ Template structure validation passed");
}