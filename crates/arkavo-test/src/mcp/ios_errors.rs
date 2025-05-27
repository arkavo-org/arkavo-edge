use serde_json::Value;
use serde_json::json;

pub enum IOSToolError {
    NoSimulatorAvailable,
    BridgeNotConnected,
    SimulatorNotBooted,
    XcodeNotInstalled,
    InvalidDeviceId,
}

impl IOSToolError {
    pub fn to_response(&self) -> Value {
        match self {
            IOSToolError::NoSimulatorAvailable => json!({
                "error": {
                    "code": "NO_SIMULATOR_AVAILABLE",
                    "message": "No iOS simulator is available. Please ensure Xcode is installed and at least one simulator is configured.",
                    "details": {
                        "suggestion": "Run 'xcrun simctl list devices' to see available simulators or 'xcrun simctl create' to create one.",
                        "documentation": "https://docs.arkavo.com/ios-setup"
                    }
                }
            }),

            IOSToolError::BridgeNotConnected => json!({
                "error": {
                    "code": "IOS_BRIDGE_NOT_CONNECTED",
                    "message": "iOS bridge is not connected. The tool is running in standalone mode without iOS integration.",
                    "details": {
                        "suggestion": "Ensure you're running on macOS with Xcode installed, or launch from an iOS XCTest environment.",
                        "current_platform": std::env::consts::OS
                    }
                }
            }),

            IOSToolError::SimulatorNotBooted => json!({
                "error": {
                    "code": "SIMULATOR_NOT_BOOTED",
                    "message": "iOS simulator exists but is not booted.",
                    "details": {
                        "suggestion": "Run 'xcrun simctl boot <device-id>' to start the simulator.",
                        "command": "xcrun simctl list devices"
                    }
                }
            }),

            IOSToolError::XcodeNotInstalled => json!({
                "error": {
                    "code": "XCODE_NOT_INSTALLED",
                    "message": "Xcode or Xcode Command Line Tools are not installed.",
                    "details": {
                        "suggestion": "Install Xcode from the App Store or run 'xcode-select --install' for command line tools.",
                        "platform": std::env::consts::OS
                    }
                }
            }),

            IOSToolError::InvalidDeviceId => json!({
                "error": {
                    "code": "INVALID_DEVICE_ID",
                    "message": "The specified device ID is invalid or device not found.",
                    "details": {
                        "suggestion": "Use 'xcrun simctl list devices' to find valid device IDs."
                    }
                }
            }),
        }
    }
}

pub fn check_ios_availability() -> Result<(), IOSToolError> {
    // Check platform
    if std::env::consts::OS != "macos" {
        return Err(IOSToolError::BridgeNotConnected);
    }

    // Check if xcrun is available
    match std::process::Command::new("xcrun")
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {}
        _ => return Err(IOSToolError::XcodeNotInstalled),
    }

    // Check for booted simulator
    match std::process::Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("(Booted)") {
                Ok(())
            } else {
                // Check if any simulators exist
                match std::process::Command::new("xcrun")
                    .args(["simctl", "list", "devices"])
                    .output()
                {
                    Ok(list_output) => {
                        let list_stdout = String::from_utf8_lossy(&list_output.stdout);
                        if list_stdout.contains("iPhone") || list_stdout.contains("iPad") {
                            Err(IOSToolError::SimulatorNotBooted)
                        } else {
                            Err(IOSToolError::NoSimulatorAvailable)
                        }
                    }
                    Err(_) => Err(IOSToolError::NoSimulatorAvailable),
                }
            }
        }
        Err(_) => Err(IOSToolError::XcodeNotInstalled),
    }
}
