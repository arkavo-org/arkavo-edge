use crate::{Result, TestError};
use std::process::{Command, Output};

/// Wrapper for xcodebuild commands that prevents system prompts
pub struct XcodebuildWrapper;

impl XcodebuildWrapper {
    /// Check if xcodebuild is available without triggering system prompts
    pub fn is_available() -> bool {
        // First check if Xcode tools are available via xcode_version
        if !super::xcode_version::XcodeVersion::is_available() {
            return false;
        }

        // Double-check by looking for xcodebuild in PATH
        match Command::new("which").arg("xcodebuild").output() {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// Execute xcodebuild command with safeguards against system prompts
    pub fn execute(args: &[&str]) -> Result<Output> {
        // Check availability first
        if !Self::is_available() {
            return Err(TestError::Mcp(format!(
                "xcodebuild not available. {}",
                super::xcode_version::XcodeVersion::require_xcode_message()
            )));
        }

        // Set environment variables to prevent installation prompts
        let mut cmd = Command::new("xcodebuild");
        cmd.args(args);

        // These environment variables help prevent system prompts
        cmd.env("COMMAND_LINE_INSTALL", "0");
        cmd.env("GCC_VERSION", "");

        // Execute the command
        match cmd.output() {
            Ok(output) => {
                // Check for common error patterns that might trigger prompts
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("license") || stderr.contains("agreement") {
                    Err(TestError::Mcp(
                        "Xcode license agreement required. Please run 'sudo xcodebuild -license accept' manually.".to_string()
                    ))
                } else if stderr.contains("xcrun: error") || stderr.contains("not found") {
                    Err(TestError::Mcp(format!(
                        "Xcode command line tools error. {}",
                        super::xcode_version::XcodeVersion::require_xcode_message()
                    )))
                } else {
                    Ok(output)
                }
            }
            Err(e) => {
                // If xcodebuild can't be executed at all, provide helpful error
                Err(TestError::Mcp(format!(
                    "Failed to execute xcodebuild: {}. {}",
                    e,
                    super::xcode_version::XcodeVersion::require_xcode_message()
                )))
            }
        }
    }

    /// Try to execute xcodebuild, returning None if not available
    pub fn try_execute(args: &[&str]) -> Option<Output> {
        if !Self::is_available() {
            return None;
        }

        Self::execute(args).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xcodebuild_availability_check() {
        // This test verifies the availability check doesn't trigger prompts
        let available = XcodebuildWrapper::is_available();
        // We don't assert the result as it depends on the system
        // Just ensure it doesn't crash or trigger prompts
        println!("xcodebuild available: {}", available);
    }
}
