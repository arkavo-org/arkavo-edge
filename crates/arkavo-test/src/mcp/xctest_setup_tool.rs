use super::device_manager::DeviceManager;
use super::ios_tools::UiInteractionKit;
use super::server::{Tool, ToolSchema};
use super::xctest_compiler::XCTestCompiler;
use super::xctest_unix_bridge::XCTestUnixBridge;
use super::xctest_verifier::XCTestVerifier;
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct XCTestSetupKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl XCTestSetupKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "setup_xcuitest".to_string(),
                description: "REQUIRED BEFORE TEXT-BASED UI INTERACTION: Initialize XCUITest to enable finding and tapping UI elements by their visible text. MUST provide target_app_bundle_id parameter with the bundle ID of the app you want to test (e.g. 'com.example.app'). Without running this first, all text-based taps will fail with XCUITEST_NOT_AVAILABLE. Call this once per device before any ui_interaction with text targets.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "target_app_bundle_id": {
                            "type": "string",
                            "description": "Bundle ID of the app to test (e.g. com.example.app). Required for text-based UI interaction."
                        },
                        "force_reinstall": {
                            "type": "boolean",
                            "description": "Force reinstall even if already setup",
                            "default": false
                        }
                    }
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for XCTestSetupKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        eprintln!("[XCTestSetupKit] Starting XCUITest setup...");

        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    return Ok(serde_json::json!({
                        "success": false,
                        "error": {
                            "code": "NO_ACTIVE_DEVICE",
                            "message": "No active device set. Use device_management tool first."
                        }
                    }));
                }
            }
        };

        let force_reinstall = params
            .get("force_reinstall")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let target_app_bundle_id = params
            .get("target_app_bundle_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Check if XCUITest is already available and functional
        if !force_reinstall {
            eprintln!(
                "[XCTestSetupKit] Checking XCTest status for device {}...",
                device_id
            );
            let verification_status = XCTestVerifier::verify_device(&device_id).await?;

            if verification_status.is_functional {
                let global_bridge = UiInteractionKit::get_global_xctest_bridge();
                if global_bridge.read().await.is_some() {
                    eprintln!("[XCTestSetupKit] XCUITest already functional on device");
                    return Ok(serde_json::json!({
                        "success": true,
                        "status": "already_setup",
                        "message": "XCUITest is already available and functional",
                        "device_status": verification_status,
                        "capabilities": [
                            "Text-based element finding: {\"action\":\"tap\",\"target\":{\"text\":\"Button Label\"}}",
                            "Accessibility ID support: {\"action\":\"tap\",\"target\":{\"accessibility_id\":\"element_id\"}}",
                            "10-second element wait timeout",
                            "Automatic retry on element not found"
                        ],
                        "note": "Use force_reinstall:true to reinstall XCUITest"
                    }));
                }
            }
        }

        // Compile XCTest bundle
        eprintln!("[XCTestSetupKit] Compiling XCTest bundle...");
        let compiler = match XCTestCompiler::new() {
            Ok(c) => c,
            Err(e) => {
                return Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "COMPILER_INIT_FAILED",
                        "message": format!("Failed to initialize XCTest compiler: {}", e),
                        "possible_causes": [
                            "Xcode not installed",
                            "Xcode command line tools not installed",
                            "Missing permissions"
                        ],
                        "solutions": [
                            "Install Xcode from App Store",
                            "Run: xcode-select --install",
                            "Run: sudo xcode-select --switch /Applications/Xcode.app",
                            "Ensure you have accepted Xcode license: sudo xcodebuild -license accept"
                        ]
                    }
                }));
            }
        };

        let bundle_path = match compiler.get_xctest_bundle() {
            Ok(path) => path,
            Err(e) => {
                return Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "COMPILATION_FAILED",
                        "message": format!("Failed to compile XCTest bundle: {}", e),
                        "details": "The XCTest runner Swift code could not be compiled",
                        "possible_causes": [
                            "Swift compiler issues",
                            "Missing iOS SDK",
                            "Syntax errors in template"
                        ]
                    }
                }));
            }
        };

        eprintln!(
            "[XCTestSetupKit] Bundle compiled at: {}",
            bundle_path.display()
        );

        // Install to simulator
        eprintln!(
            "[XCTestSetupKit] Installing bundle to simulator {}...",
            device_id
        );
        if let Err(e) = compiler.install_to_simulator(&device_id, &bundle_path) {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "INSTALLATION_FAILED",
                    "message": format!("Failed to install XCTest bundle: {}", e),
                    "device_id": device_id,
                    "possible_causes": [
                        "Simulator not booted",
                        "Insufficient permissions",
                        "Simulator in bad state"
                    ],
                    "solutions": [
                        "Ensure simulator is booted: xcrun simctl boot [device_id]",
                        "Try restarting the simulator",
                        "Reset simulator: xcrun simctl erase [device_id]"
                    ]
                }
            }));
        }

        // Test the installation
        eprintln!("[XCTestSetupKit] Testing XCUITest connection...");
        let socket_path = compiler.socket_path().to_path_buf();
        let mut bridge = XCTestUnixBridge::with_socket_path(socket_path.clone());

        // Start the bridge
        if let Err(e) = bridge.start().await {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "BRIDGE_START_FAILED",
                    "message": format!("Failed to start XCTest bridge: {}", e),
                    "details": "The Unix socket server could not be started"
                }
            }));
        }

        // Launch the target app first if specified
        if let Some(ref app_id) = target_app_bundle_id {
            eprintln!(
                "[XCTestSetupKit] Checking if target app is installed: {}",
                app_id
            );

            // First check if app is installed
            let list_apps_result = std::process::Command::new("xcrun")
                .args(["simctl", "listapps", &device_id])
                .output();

            let app_installed = match list_apps_result {
                Ok(output) => {
                    let apps_str = String::from_utf8_lossy(&output.stdout);
                    apps_str.contains(app_id)
                }
                Err(_) => false,
            };

            if !app_installed {
                return Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "APP_NOT_INSTALLED",
                        "message": format!("Target app '{}' is not installed on the simulator", app_id),
                        "details": "Install the app first using 'app_management' tool with action 'install', then run setup_xcuitest again.",
                        "hint": "Use app_diagnostic tool to check which apps are installed."
                    }
                }));
            }

            eprintln!("[XCTestSetupKit] Launching target app: {}", app_id);
            let launch_result = std::process::Command::new("xcrun")
                .args(["simctl", "launch", &device_id, app_id])
                .output();

            match launch_result {
                Ok(output) => {
                    if output.status.success() {
                        eprintln!("[XCTestSetupKit] Target app launched successfully");
                    } else {
                        let error_msg = String::from_utf8_lossy(&output.stderr);
                        eprintln!(
                            "[XCTestSetupKit] Failed to launch target app: {}",
                            error_msg
                        );

                        return Ok(serde_json::json!({
                            "success": false,
                            "error": {
                                "code": "APP_LAUNCH_FAILED",
                                "message": format!("Failed to launch target app '{}'", app_id),
                                "details": format!("Launch error: {}", error_msg),
                                "hint": "Make sure the app is properly installed and not corrupted."
                            }
                        }));
                    }
                }
                Err(e) => {
                    eprintln!("[XCTestSetupKit] Could not launch target app: {}", e);
                    return Ok(serde_json::json!({
                        "success": false,
                        "error": {
                            "code": "APP_LAUNCH_ERROR",
                            "message": format!("Could not launch target app '{}'", app_id),
                            "details": format!("System error: {}", e)
                        }
                    }));
                }
            }

            // Give app time to start and come to foreground
            std::thread::sleep(std::time::Duration::from_secs(3));
        }

        // Launch the test host app
        eprintln!("[XCTestSetupKit] Launching test host app...");
        if let Err(e) = compiler.launch_test_host(&device_id, target_app_bundle_id.as_deref()) {
            let error_str = e.to_string();

            // Check if the error is due to app not being installed
            if error_str.contains("failed to launch")
                || error_str.contains("FBSOpenApplicationServiceErrorDomain")
            {
                return Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "APP_NOT_INSTALLED",
                        "message": format!("The app '{}' is not installed on the simulator", target_app_bundle_id.as_deref().unwrap_or("unknown")),
                        "details": "Install the app first using 'app_management' tool with action 'install', then run setup_xcuitest again.",
                        "hint": "Use app_diagnostic tool to check which apps are installed.",
                        "raw_error": error_str
                    }
                }));
            }

            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "TEST_RUN_FAILED",
                    "message": format!("Failed to run XCTest bundle: {}", e),
                    "details": "The test bundle could not be executed on the simulator"
                }
            }));
        }

        // Connect to the test runner (as a client)
        eprintln!("[XCTestSetupKit] Connecting to test runner...");
        eprintln!(
            "[XCTestSetupKit] Socket path: {}",
            compiler.socket_path().display()
        );

        // Give the Swift side time to start up and bind to the socket
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        match tokio::time::timeout(
            tokio::time::Duration::from_secs(30), // Increased timeout
            bridge.connect_to_runner(),
        )
        .await
        {
            Ok(Ok(())) => {
                eprintln!("[XCTestSetupKit] Successfully connected to test runner!");

                // Test the connection with a ping
                let test_result = bridge.send_ping().await;
                if let Err(e) = test_result {
                    eprintln!("[XCTestSetupKit] Warning: Ping test failed: {}", e);
                    return Ok(serde_json::json!({
                        "success": false,
                        "error": {
                            "code": "CONNECTION_TEST_FAILED",
                            "message": "Connected to test runner but communication failed",
                            "details": format!("Ping error: {}", e),
                            "hint": "The test host may have crashed or the socket connection is not working properly"
                        }
                    }));
                }

                eprintln!("[XCTestSetupKit] Connection test passed");

                // Store the bridge in the global storage so ui_interaction can use it
                let global_bridge = UiInteractionKit::get_global_xctest_bridge();
                let bridge_arc = Arc::new(Mutex::new(bridge));
                *global_bridge.write().await = Some(bridge_arc);
                eprintln!("[XCTestSetupKit] Global XCTest bridge updated");

                // Don't run the verifier - it's checking for things that don't apply to our bridge approach
                // let final_status = XCTestVerifier::verify_device(&device_id).await?;

                Ok(serde_json::json!({
                    "success": true,
                    "status": "setup_complete",
                    "message": "XCUITest is now available for text-based UI interaction",
                    "device_id": device_id,
                    "target_app_bundle_id": target_app_bundle_id,
                    "capabilities": {
                        "text_based_tap": "Use {\"action\":\"tap\",\"target\":{\"text\":\"Button Label\"}}",
                        "accessibility_id": "Use {\"action\":\"tap\",\"target\":{\"accessibility_id\":\"element_id\"}}",
                        "element_wait": "Automatically waits up to 10 seconds for elements",
                        "supported_actions": ["tap", "type_text", "clear_text", "swipe"]
                    },
                    "next_steps": [
                        "You can now use ui_interaction with text-based targets",
                        "Example: {\"action\":\"tap\",\"target\":{\"text\":\"Get Started\"}}",
                        "The system will find and tap elements by their visible text"
                    ]
                }))
            }
            Ok(Err(e)) => {
                eprintln!("[XCTestSetupKit] Connection failed: {}", e);
                Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "CONNECTION_FAILED",
                        "message": format!("XCTest runner failed to connect: {}", e),
                        "details": "The test bundle started but didn't establish connection. The test host may have crashed.",
                        "hint": "Check the simulator console logs for crash reports."
                    }
                }))
            }
            Err(_) => {
                eprintln!("[XCTestSetupKit] Connection timeout after 30 seconds");
                Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "CONNECTION_TIMEOUT",
                        "message": "Timeout waiting for XCTest runner to connect",
                        "details": "The test bundle may have crashed or failed to start properly after 30 seconds.",
                        "hint": "Check if the test host app (ArkavoTestHost) is running on the simulator."
                    }
                }))
            }
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
