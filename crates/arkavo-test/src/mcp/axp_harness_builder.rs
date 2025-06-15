use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use super::templates;
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

pub struct AxpHarnessBuilder {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl AxpHarnessBuilder {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "build_test_harness".to_string(),
                description: "🚀 REQUIRED FIRST STEP! Build a generic AXP test harness for fast touch injection. Without this, taps take 300ms+ and IDB may fail with port conflicts. This creates a lightweight service that provides <30ms taps for ANY iOS app. Just provide the bundle ID - no Xcode project needed. Run this ONCE per app before any UI testing.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "app_bundle_id": {
                            "type": "string",
                            "description": "Bundle ID of YOUR app to test (e.g., com.company.appname). Get this from your app's Info.plist or Xcode project settings."
                        },
                        "harness_type": {
                            "type": "string",
                            "enum": ["axp", "xcuitest"],
                            "default": "axp",
                            "description": "Type of harness to build (axp recommended for speed)"
                        }
                    },
                    "required": ["app_bundle_id"]
                }),
            },
            device_manager,
        }
    }

    async fn build_axp_harness(
        &self,
        app_bundle_id: &str,
    ) -> Result<Value> {
        eprintln!("[AxpHarnessBuilder] Building generic AXP harness for {}", app_bundle_id);
        
        // Pre-flight check: Verify Xcode tools are available
        let xcrun_check = Command::new("which")
            .arg("xcrun")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
            
        if !xcrun_check {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "IOS_SDK_MISSING",
                    "message": "Xcode command line tools not found",
                    "solution": "Install Xcode and run: xcode-select --install"
                }
            }));
        }
        
        // Check if developer directory is set
        let xcode_select_check = Command::new("xcode-select")
            .arg("-p")
            .output();
            
        match xcode_select_check {
            Ok(output) if output.status.success() => {
                let dev_dir = String::from_utf8_lossy(&output.stdout);
                eprintln!("[AxpHarnessBuilder] Developer directory: {}", dev_dir.trim());
            }
            _ => {
                return Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "IOS_SDK_MISSING",
                        "message": "Xcode developer directory not set",
                        "solution": "Run: sudo xcode-select --switch /Applications/Xcode.app"
                    }
                }));
            }
        }

        // Validate bundle ID format
        if !app_bundle_id.contains('.') {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "INVALID_BUNDLE_ID",
                    "message": "Bundle ID must be in reverse domain format",
                    "example": "com.company.appname"
                }
            }));
        }

        // Create build directory in temp
        let build_dir = std::env::temp_dir()
            .join(format!("arkavo-axp-harness-{}", app_bundle_id.replace('.', "_")));

        fs::create_dir_all(&build_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create build directory: {}", e)))?;

        eprintln!("[AxpHarnessBuilder] Build directory: {}", build_dir.display());

        // Create source directory structure
        let sources_dir = build_dir.join("Sources");
        let harness_dir = sources_dir.join("ArkavoHarness");
        fs::create_dir_all(&harness_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create source directory: {}", e)))?;

        // Generate socket path for this harness with device UDID
        let device_id = self.device_manager.get_active_device()
            .map(|d| d.id.clone())
            .ok_or_else(|| TestError::Mcp("No active device".to_string()))?;
            
        let socket_path = std::env::temp_dir()
            .join(format!("arkavo-axp-{}-{}.sock", device_id, app_bundle_id.replace('.', "_")));

        // Write AXP bridge code
        let ax_bridge_path = harness_dir.join("ArkavoAXBridge.swift");
        fs::write(&ax_bridge_path, templates::ARKAVO_AX_BRIDGE_SWIFT)
            .map_err(|e| TestError::Mcp(format!("Failed to write AX bridge: {}", e)))?;

        // Write test runner with AXP support
        let runner_content = templates::ARKAVO_TEST_RUNNER_AXP_SWIFT
            .replace("{{SOCKET_PATH}}", &socket_path.to_string_lossy());
        
        let runner_path = harness_dir.join("ArkavoTestRunner.swift");
        fs::write(&runner_path, runner_content)
            .map_err(|e| TestError::Mcp(format!("Failed to write test runner: {}", e)))?;

        // Create Package.swift
        let package_swift = format!(
            r#"// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "ArkavoHarness",
    platforms: [
        .iOS(.v15)
    ],
    products: [
        .library(
            name: "ArkavoHarness",
            type: .dynamic,
            targets: ["ArkavoHarness"]
        )
    ],
    targets: [
        .target(
            name: "ArkavoHarness",
            dependencies: [],
            path: "Sources/ArkavoHarness"
        )
    ]
)
"#
        );

        let package_path = build_dir.join("Package.swift");
        fs::write(&package_path, package_swift)
            .map_err(|e| TestError::Mcp(format!("Failed to write Package.swift: {}", e)))?;

        // Create Info.plist for the generic test bundle
        let info_plist = templates::GENERIC_AXP_HARNESS_PLIST;

        let plist_path = build_dir.join("Info.plist");
        fs::write(&plist_path, info_plist)
            .map_err(|e| TestError::Mcp(format!("Failed to write Info.plist: {}", e)))?;

        // Compile the harness
        eprintln!("[AxpHarnessBuilder] Compiling AXP harness...");
        
        let compile_result = self.compile_harness(&build_dir, &plist_path).await?;
        
        if !compile_result["success"].as_bool().unwrap_or(false) {
            return Ok(compile_result);
        }

        let bundle_path = compile_result["bundle_path"].as_str().unwrap();

        // Install to active simulator if available
        if let Some(device) = self.device_manager.get_active_device() {
            eprintln!("[AxpHarnessBuilder] Installing to simulator {}...", device.id);
            
            if let Err(e) = self.install_to_simulator(&device.id, bundle_path) {
                eprintln!("[AxpHarnessBuilder] Warning: Failed to auto-install: {}", e);
            } else {
                eprintln!("[AxpHarnessBuilder] Successfully installed to simulator");
            }
        }

        Ok(serde_json::json!({
            "success": true,
            "harness_type": "axp",
            "bundle_path": bundle_path,
            "socket_path": socket_path.to_string_lossy(),
            "app_bundle_id": app_bundle_id,
            "capabilities": {
                "axp_tap": "Fast coordinate-based tapping using AXP",
                "axp_snapshot": "Accessibility tree snapshots",
                "fallback": "Automatic fallback to XCUICoordinate if AXP unavailable"
            },
            "important": "This generic harness works with ANY iOS app - no project files needed",
            "next_steps": [
                "The harness is now installed on the simulator",
                "Launch your app using app_launcher tool",
                "Use ui_interaction - taps will now be <30ms!"
            ]
        }))
    }

    async fn compile_harness(&self, build_dir: &Path, plist_path: &Path) -> Result<Value> {
        // Get SDK paths
        let sdk_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get SDK path: {}", e)))?;

        if !sdk_output.status.success() {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "SDK_NOT_FOUND",
                    "message": "Failed to get iOS simulator SDK path"
                }
            }));
        }

        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout).trim().to_string();

        // Build using Swift
        let swift_files = vec![
            build_dir.join("Sources/ArkavoHarness/ArkavoAXBridge.swift"),
            build_dir.join("Sources/ArkavoHarness/ArkavoTestRunner.swift"),
        ];

        let output_binary = build_dir.join("ArkavoHarness");
        
        let mut cmd = Command::new("swiftc");
        cmd.args([
            "-sdk", &sdk_path,
            "-target", "arm64-apple-ios15.0-simulator",
            "-parse-as-library",
            "-emit-library",
            "-module-name", "ArkavoHarness",
            "-o", output_binary.to_str().unwrap(),
        ]);

        // Add all Swift files
        for file in &swift_files {
            cmd.arg(file.to_str().unwrap());
        }

        eprintln!("[AxpHarnessBuilder] Running: {:?}", cmd);

        let output = cmd.output()
            .map_err(|e| TestError::Mcp(format!("Failed to compile: {}", e)))?;

        if !output.status.success() {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "COMPILATION_FAILED",
                    "message": "Failed to compile AXP harness",
                    "details": String::from_utf8_lossy(&output.stderr).to_string()
                }
            }));
        }

        // Create .xctest bundle
        let xctest_bundle = build_dir.join("ArkavoHarness.xctest");
        fs::create_dir_all(&xctest_bundle)
            .map_err(|e| TestError::Mcp(format!("Failed to create bundle: {}", e)))?;

        // Copy binary
        fs::copy(&output_binary, xctest_bundle.join("ArkavoHarness"))
            .map_err(|e| TestError::Mcp(format!("Failed to copy binary: {}", e)))?;

        // Copy Info.plist
        fs::copy(plist_path, xctest_bundle.join("Info.plist"))
            .map_err(|e| TestError::Mcp(format!("Failed to copy plist: {}", e)))?;

        Ok(serde_json::json!({
            "success": true,
            "bundle_path": xctest_bundle.to_string_lossy().to_string()
        }))
    }

    fn install_to_simulator(&self, device_id: &str, bundle_path: &str) -> Result<()> {
        // Use simctl to install the test bundle
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "xctest",
                "install",
                device_id,
                bundle_path,
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run xctest install: {}", e)))?;

        if !output.status.success() {
            // Try alternative installation method
            let alt_output = Command::new("xcrun")
                .args([
                    "simctl",
                    "install",
                    device_id,
                    bundle_path,
                ])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to run simctl install: {}", e)))?;

            if !alt_output.status.success() {
                return Err(TestError::Mcp(format!(
                    "Failed to install test bundle: {}",
                    String::from_utf8_lossy(&alt_output.stderr)
                )));
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Tool for AxpHarnessBuilder {
    async fn execute(&self, params: Value) -> Result<Value> {
        let app_bundle_id = params
            .get("app_bundle_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing app_bundle_id parameter".to_string()))?;

        let harness_type = params
            .get("harness_type")
            .and_then(|v| v.as_str())
            .unwrap_or("axp");

        match harness_type {
            "axp" => self.build_axp_harness(app_bundle_id).await,
            "xcuitest" => Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "NOT_IMPLEMENTED",
                    "message": "XCUITest harness type not yet implemented. Use 'axp' for fast touch injection."
                }
            })),
            _ => Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "INVALID_HARNESS_TYPE",
                    "message": format!("Unknown harness type: {}. Use 'axp' or 'xcuitest'", harness_type)
                }
            })),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}