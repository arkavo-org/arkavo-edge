use super::device_manager::DeviceManager;
use super::ios_tools::UiInteractionKit;
use super::server::{Tool, ToolSchema};
use super::xctest_compiler::XCTestCompiler;
use super::xctest_unix_bridge::XCTestUnixBridge;
use super::xctest_verifier::XCTestVerifier;
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::process::Command;
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
                description: "⚠️ DEPRECATED - Use build_test_harness instead for fast, reliable automation. This old approach often fails with timeouts. The new AXP-based harness is 10x faster and more reliable. DO NOT USE unless specifically debugging legacy issues.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "target_app_bundle_id": {
                            "type": "string",
                            "description": "DEPRECATED: Do not use. Specifying a target app causes security restrictions. XCUITest will work with any app on the simulator without this parameter."
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

        // Ignore target_app_bundle_id even if provided - it causes security restrictions
        let _target_app_bundle_id: Option<String> = None;

        // Log warning if target_app_bundle_id was provided
        if params.get("target_app_bundle_id").is_some() {
            eprintln!(
                "[XCTestSetupKit] WARNING: target_app_bundle_id parameter is deprecated and will be ignored"
            );
            eprintln!(
                "[XCTestSetupKit] XCUITest will work with any app on the simulator without specifying a target"
            );
        }

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

        // No longer launching target app - it causes security restrictions

        // Launch the test host app without target app
        eprintln!("[XCTestSetupKit] Launching test host app...");
        if let Err(e) = compiler.launch_test_host(&device_id, None) {
            let error_str = e.to_string();

            // Check if the error is due to test host issues
            if error_str.contains("failed to launch")
                || error_str.contains("FBSOpenApplicationServiceErrorDomain")
            {
                return Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "TEST_HOST_LAUNCH_FAILED",
                        "message": "Failed to launch XCTest host app",
                        "details": "The test host app could not be launched. This might be due to a previous crash or installation issue.",
                        "hint": "Try running with force_reinstall: true, or manually uninstall com.arkavo.testhost from the simulator.",
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

                // Return to the previous app if there was one
                eprintln!("[XCTestSetupKit] Setup complete, returning to previous app...");

                // Launch the previous app or home screen to hide the test host
                let _ = Command::new("xcrun")
                    .args(["simctl", "launch", &device_id, "com.apple.springboard"])
                    .output();

                Ok(serde_json::json!({
                    "success": true,
                    "status": "setup_complete",
                    "message": "XCUITest is now available for UI automation on any app",
                    "device_id": device_id,
                    "important_note": "The ArkavoTestHost app briefly appeared with a black screen but has been moved to background. This is normal and expected.",
                    "capabilities": {
                        "coordinate_tap": "Improved coordinate-based tapping through XCUITest",
                        "text_based_tap": "Find and tap elements by visible text (when XCUITest can access the app)",
                        "accessibility_id": "Find elements by accessibility identifier",
                        "element_wait": "Automatically waits up to 10 seconds for elements",
                        "supported_actions": ["tap", "type_text", "clear_text", "swipe"],
                        "works_with": "Any app running on the simulator"
                    },
                    "next_steps": [
                        "Launch any app you want to test using app_launcher",
                        "Use ui_interaction with coordinate-based targets for reliable interaction",
                        "Example: {\"action\":\"tap\",\"target\":{\"x\":196,\"y\":680}}",
                        "Text-based targets may work depending on app accessibility"
                    ],
                    "background_info": "ArkavoTestHost runs invisibly in the background to provide UI automation capabilities"
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
