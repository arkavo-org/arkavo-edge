use super::device_manager::DeviceManager;
use super::ios_tools::UiInteractionKit;
use super::server::{Tool, ToolSchema};
use super::xctest_compiler::XCTestCompiler;
use super::xctest_unix_bridge::XCTestUnixBridge;
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
                description: "Initialize XCUITest to enable text-based UI element finding. This compiles and installs the XCUITest runner, allowing you to tap elements by their visible text or accessibility IDs instead of coordinates.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
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

        // Check if XCUITest is already available in the global bridge
        if !force_reinstall {
            let global_bridge = UiInteractionKit::get_global_xctest_bridge();
            if global_bridge.read().await.is_some() {
                eprintln!("[XCTestSetupKit] XCUITest already available in global bridge");
                return Ok(serde_json::json!({
                    "success": true,
                    "status": "already_setup",
                    "message": "XCUITest is already available",
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

        eprintln!("[XCTestSetupKit] Bundle compiled at: {}", bundle_path.display());

        // Install to simulator
        eprintln!("[XCTestSetupKit] Installing bundle to simulator {}...", device_id);
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

        // Run the test bundle
        eprintln!("[XCTestSetupKit] Running test bundle...");
        if let Err(e) = compiler.run_tests(&device_id, "com.arkavo.testrunner") {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "TEST_RUN_FAILED",
                    "message": format!("Failed to run XCTest bundle: {}", e),
                    "details": "The test bundle could not be executed on the simulator"
                }
            }));
        }

        // Wait for connection
        eprintln!("[XCTestSetupKit] Waiting for test runner to connect...");
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(10),
            bridge.wait_for_connection()
        ).await {
            Ok(Ok(())) => {
                eprintln!("[XCTestSetupKit] XCUITest setup completed successfully!");
                
                // Store the bridge in the global storage so ui_interaction can use it
                let global_bridge = UiInteractionKit::get_global_xctest_bridge();
                let bridge_arc = Arc::new(Mutex::new(bridge));
                *global_bridge.write().await = Some(bridge_arc);
                eprintln!("[XCTestSetupKit] Global XCTest bridge updated");
                
                Ok(serde_json::json!({
                    "success": true,
                    "status": "setup_complete",
                    "message": "XCUITest is now available for text-based UI interaction",
                    "device_id": device_id,
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
                Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "CONNECTION_TIMEOUT",
                        "message": format!("XCTest runner failed to connect: {}", e),
                        "details": "The test bundle started but didn't establish connection"
                    }
                }))
            }
            Err(_) => {
                Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "CONNECTION_TIMEOUT",
                        "message": "Timeout waiting for XCTest runner to connect",
                        "details": "The test bundle may have crashed or failed to start properly"
                    }
                }))
            }
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}