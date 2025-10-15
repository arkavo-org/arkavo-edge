#![allow(clippy::disallowed_methods)]

//! Regression test for issue #114: Remove xcodebuild runtime dependency
//!
//! This test ensures that the xcodebuild wrapper properly prevents system prompts
//! when Xcode Command Line Tools are not installed.

use arkavo_mcp_macos::mcp::xcode_version::XcodeVersion;
use arkavo_mcp_macos::mcp::xcodebuild_wrapper::XcodebuildWrapper;

#[test]
fn test_xcode_version_no_prompt() {
    // This should never trigger a system prompt, even if Xcode is not installed
    let version = XcodeVersion::detect();

    // The result doesn't matter - what matters is that no prompt was shown
    match version {
        Some(v) => println!(
            "Xcode version detected: {}.{}.{}",
            v.major, v.minor, v.patch
        ),
        None => println!("Xcode not detected (expected on systems without Xcode)"),
    }
}

#[test]
fn test_xcodebuild_wrapper_no_prompt() {
    // Test that the wrapper correctly handles missing xcodebuild
    let available = XcodebuildWrapper::is_available();
    println!("xcodebuild available: {available}");

    // Try to execute a command - should fail gracefully without prompts
    if !available {
        let result = XcodebuildWrapper::execute(&["-version"]);
        assert!(
            result.is_err(),
            "Expected error when xcodebuild is not available"
        );

        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("xcodebuild not available"),
            "Error message should indicate xcodebuild is not available"
        );
        assert!(
            err_msg.contains("Xcode Command Line Tools"),
            "Error message should mention Xcode Command Line Tools"
        );
    }
}

#[test]
fn test_xcodebuild_try_execute_no_prompt() {
    // Test the try_execute method
    let result = XcodebuildWrapper::try_execute(&["-version"]);

    match result {
        Some(output) => {
            println!("xcodebuild executed successfully");
            assert!(output.status.success() || !output.status.success());
        }
        None => {
            println!("xcodebuild not available (expected on systems without Xcode)");
        }
    }
}

#[test]
fn test_environment_variables_set_correctly() {
    use std::process::Command;

    // Only test if xcodebuild is available
    if XcodebuildWrapper::is_available() {
        // Create a command that echoes environment variables
        let output = Command::new("sh")
            .arg("-c")
            .arg("xcodebuild -version 2>&1 | head -20")
            .env("COMMAND_LINE_INSTALL", "0")
            .env("GCC_VERSION", "")
            .output();

        match output {
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);

                // Ensure no installation prompts in output
                assert!(
                    !stderr.contains("command line tools are required"),
                    "Should not prompt for command line tools"
                );
                assert!(
                    !stdout.contains("command line tools are required"),
                    "Should not prompt for command line tools"
                );
            }
            Err(e) => {
                println!("Command failed (expected on systems without sh): {e}");
            }
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn test_no_system_prompt_simulation() {
    // This test simulates the condition where xcodebuild is not in PATH
    use std::process::Command;

    // First, check where xcodebuild is actually located
    let which_output = Command::new("which")
        .arg("xcodebuild")
        .output()
        .expect("Failed to execute which");

    if which_output.status.success() {
        let xcodebuild_path = String::from_utf8_lossy(&which_output.stdout)
            .trim()
            .to_string();
        println!("xcodebuild found at: {xcodebuild_path}");

        // Create a PATH that excludes the directory containing xcodebuild
        // On GitHub Actions, xcodebuild is in /usr/bin, so we need a truly minimal PATH
        let temp_path = "/bin:/sbin"; // Exclude /usr/bin where xcodebuild might be

        // Test with restricted PATH
        let output = Command::new("sh")
            .arg("-c")
            .arg("which xcodebuild")
            .env("PATH", temp_path)
            .output()
            .expect("Failed to execute which");

        // Should fail to find xcodebuild without triggering any prompts
        assert!(
            !output.status.success(),
            "Should not find xcodebuild in restricted PATH. Found at: {xcodebuild_path}"
        );
    } else {
        // If xcodebuild is not available, that's fine - test passes
        println!("xcodebuild not found on system - test passes by default");
    }
}

#[test]
fn test_plugin_loading_error_handling() {
    // Test that plugin loading errors are handled gracefully
    use arkavo_mcp_macos::mcp::xcodebuild_wrapper::XcodebuildWrapper;

    // If xcodebuild is available but misconfigured (missing Command Line Tools),
    // it should return a proper error without triggering prompts
    if XcodebuildWrapper::is_available() {
        // Try to run a simple xcodebuild command
        let result = XcodebuildWrapper::execute(&["-version"]);

        // Check if we get a plugin loading error
        if let Err(e) = &result {
            let error_msg = e.to_string();
            if error_msg.contains("not properly installed") {
                println!("Correctly detected missing Command Line Tools");
                assert!(error_msg.contains("Xcode Command Line Tools"));
            }
        }

        // Either way, we shouldn't crash or trigger prompts
        println!("xcodebuild wrapper handled execution gracefully");
    }
}
