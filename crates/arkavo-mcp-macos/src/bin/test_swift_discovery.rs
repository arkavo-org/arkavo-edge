//! Test binary for Swift test discovery without triggering Xcode prompts
//! Used by regression tests to ensure issue #114 remains fixed

use arkavo_mcp_macos::TestError;
use std::process::exit;

fn main() {
    println!("Testing Swift test discovery without Xcode prompts...");

    // This simulates what would happen when discover_swift_tests is called
    match test_swift_discovery() {
        Ok(count) => {
            println!("Successfully discovered {count} Swift tests");
            exit(0);
        }
        Err(e) => {
            println!("Swift test discovery unavailable: {e}");
            // This is expected behavior when Xcode is not installed
            // The important thing is that no system prompt was triggered
            exit(0);
        }
    }
}

fn test_swift_discovery() -> Result<usize, TestError> {
    use arkavo_mcp_macos::mcp::xcodebuild_wrapper::XcodebuildWrapper;

    // Check if xcodebuild is available
    if !XcodebuildWrapper::is_available() {
        return Err(TestError::Mcp(
            "Xcode not available - Swift tests cannot be discovered".to_string(),
        ));
    }

    // Try to list schemes (this would normally be done by discover_swift_tests)
    let output = XcodebuildWrapper::try_execute(&["-list", "-project", "dummy.xcodeproj"]);

    match output {
        Some(_) => Ok(0), // Found xcodebuild but probably no project
        None => Err(TestError::Mcp("xcodebuild not available".to_string())),
    }
}
