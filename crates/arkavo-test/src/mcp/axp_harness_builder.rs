use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use super::templates;
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

mod axp;

pub struct AxpHarnessBuilder {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
    socket_manager: axp::socket::SocketManager,
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
            socket_manager: axp::socket::SocketManager::new(),
        }
    }

    /// Get iOS 26 beta compilation guidance
    pub fn get_ios26_beta_guidance() -> &'static str {
        axp::constants::IOS26_BETA_COMPILATION_GUIDANCE
    }

    async fn build_axp_harness(&self, app_bundle_id: &str) -> Result<Value> {
        eprintln!(
            "[AxpHarnessBuilder] Building generic AXP harness for {}",
            app_bundle_id
        );

        // Pre-flight checks
        if let Err(response) = self.verify_xcode_tools() {
            return Ok(response);
        }

        // Validate bundle ID format
        if !app_bundle_id.contains('.') {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": axp::constants::error_codes::INVALID_BUNDLE_ID,
                    "message": "Bundle ID must be in reverse domain format",
                    "example": "com.company.appname"
                }
            }));
        }

        // Get device info
        let device = self
            .device_manager
            .get_active_device()
            .ok_or_else(|| TestError::Mcp("No active device".to_string()))?;

        let device_id = device.id.clone();

        // Parse iOS version
        let ios_version = axp::version::IosVersion::parse(&device.runtime)
            .unwrap_or(axp::version::IosVersion {
                major: 18,
                minor: 0,
                patch: None,
                is_beta: false,
            });

        eprintln!(
            "[AxpHarnessBuilder] Device runtime: {} (parsed as {})",
            device.runtime,
            ios_version.display_string()
        );

        // Create build directory
        let build_dir = self.create_build_directory(app_bundle_id)?;

        // Create source structure
        self.create_source_structure(&build_dir)?;

        // Get socket path
        let socket_path = self.socket_manager.get_socket_path(&device_id, app_bundle_id);

        // Write Swift files based on iOS version
        self.write_swift_files(&build_dir, &socket_path, &ios_version)?;

        // Create Package.swift
        self.create_package_swift(&build_dir)?;

        // Create Info.plist
        let plist_path = self.create_info_plist(&build_dir)?;

        // Compile the harness
        let sim_version = ios_version.major.to_string();
        let compile_result = axp::compilation::HarnessCompiler::compile_with_spm(&build_dir, &plist_path, &sim_version).await?;

        if !compile_result["success"].as_bool().unwrap_or(false) {
            return Ok(compile_result);
        }

        let bundle_path = compile_result["bundle_path"].as_str().unwrap();

        // Install to simulator
        if let Err(e) = self.install_to_simulator(&device_id, bundle_path) {
            eprintln!("[AxpHarnessBuilder] Warning: Failed to auto-install: {}", e);
        } else {
            eprintln!("[AxpHarnessBuilder] Successfully installed to simulator");
        }

        // Build result with detailed capabilities
        Ok(self.build_success_response(
            bundle_path,
            &socket_path,
            app_bundle_id,
            &ios_version,
        ))
    }

    fn verify_xcode_tools(&self) -> std::result::Result<(), Value> {
        // Check xcrun
        let xcrun_check = Command::new("which")
            .arg("xcrun")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !xcrun_check {
            return Err(serde_json::json!({
                "success": false,
                "error": {
                    "code": axp::constants::error_codes::IOS_SDK_MISSING,
                    "message": "Xcode command line tools not found",
                    "solution": "Install Xcode and run: xcode-select --install"
                }
            }));
        }

        // Check developer directory
        let xcode_select_check = Command::new("xcode-select").arg("-p").output();

        match xcode_select_check {
            Ok(output) if output.status.success() => {
                let dev_dir = String::from_utf8_lossy(&output.stdout);
                eprintln!(
                    "[AxpHarnessBuilder] Developer directory: {}",
                    dev_dir.trim()
                );
                Ok(())
            }
            _ => Err(serde_json::json!({
                "success": false,
                "error": {
                    "code": axp::constants::error_codes::IOS_SDK_MISSING,
                    "message": "Xcode developer directory not set",
                    "solution": "Run: sudo xcode-select --switch /Applications/Xcode.app"
                }
            })),
        }
    }

    fn create_build_directory(&self, app_bundle_id: &str) -> Result<PathBuf> {
        let arkavo_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".arkavo")
            .join("axp-harnesses");

        let build_dir = arkavo_dir.join(format!(
            "arkavo-axp-harness-{}",
            app_bundle_id.replace('.', "_")
        ));

        fs::create_dir_all(&build_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create build directory: {}", e)))?;

        eprintln!(
            "[AxpHarnessBuilder] Build directory: {}",
            build_dir.display()
        );

        Ok(build_dir)
    }

    fn create_source_structure(&self, build_dir: &Path) -> Result<PathBuf> {
        let sources_dir = build_dir.join("Sources");
        let harness_dir = sources_dir.join("ArkavoHarness");
        fs::create_dir_all(&harness_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create source directory: {}", e)))?;
        Ok(harness_dir)
    }

    fn write_swift_files(
        &self,
        build_dir: &Path,
        socket_path: &Path,
        ios_version: &axp::version::IosVersion,
    ) -> Result<()> {
        let harness_dir = build_dir.join("Sources").join("ArkavoHarness");

        // Write AX bridge based on iOS version
        let ax_bridge_path = harness_dir.join("ArkavoAXBridge.swift");
        if ios_version.is_ios26_or_later() {
            eprintln!("[AxpHarnessBuilder] Using iOS 26 enhanced bridge with symbol discovery");
            fs::write(&ax_bridge_path, templates::ARKAVO_AX_BRIDGE_IOS26_SWIFT)
                .map_err(|e| TestError::Mcp(format!("Failed to write iOS 26 AX bridge: {}", e)))?;
        } else {
            fs::write(&ax_bridge_path, templates::ARKAVO_AX_BRIDGE_SWIFT)
                .map_err(|e| TestError::Mcp(format!("Failed to write AX bridge: {}", e)))?;
        }

        // Write test runner based on iOS version
        let runner_path = harness_dir.join("ArkavoTestRunner.swift");
        if ios_version.is_ios26_or_later() {
            eprintln!("[AxpHarnessBuilder] Using minimal runner for iOS 26 beta compatibility");
            let runner_content = templates::ARKAVO_TEST_RUNNER_MINIMAL_SWIFT
                .replace("{{SOCKET_PATH}}", &socket_path.to_string_lossy());
            fs::write(&runner_path, runner_content)
                .map_err(|e| TestError::Mcp(format!("Failed to write minimal test runner: {}", e)))?;
        } else {
            let runner_content = templates::ARKAVO_TEST_RUNNER_AXP_SWIFT
                .replace("{{SOCKET_PATH}}", &socket_path.to_string_lossy());
            fs::write(&runner_path, runner_content)
                .map_err(|e| TestError::Mcp(format!("Failed to write test runner: {}", e)))?;
        }

        Ok(())
    }

    fn create_package_swift(&self, build_dir: &Path) -> Result<()> {
        let package_swift = r#"// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "ArkavoHarness",
    platforms: [
        .iOS(.v15),
        .macOS(.v14)  // Match the XCTest framework requirements
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
"#;

        let package_path = build_dir.join("Package.swift");
        fs::write(&package_path, package_swift)
            .map_err(|e| TestError::Mcp(format!("Failed to write Package.swift: {}", e)))?;

        Ok(())
    }

    fn create_info_plist(&self, build_dir: &Path) -> Result<PathBuf> {
        let info_plist = templates::GENERIC_AXP_HARNESS_PLIST;
        let plist_path = build_dir.join("Info.plist");
        fs::write(&plist_path, info_plist)
            .map_err(|e| TestError::Mcp(format!("Failed to write Info.plist: {}", e)))?;
        Ok(plist_path)
    }

    fn install_to_simulator(&self, device_id: &str, bundle_path: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "install",
                device_id,
                bundle_path,
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run xcrun simctl install: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TestError::Mcp(format!(
                "Failed to install harness: {}",
                stderr
            )));
        }

        Ok(())
    }

    fn build_success_response(
        &self,
        bundle_path: &str,
        socket_path: &Path,
        app_bundle_id: &str,
        ios_version: &axp::version::IosVersion,
    ) -> Value {
        let mut capabilities = vec![
            "fast_tap (<30ms)",
            "axp_bridge",
            "unix_socket_control",
            "screenshot_capture",
        ];

        if ios_version.is_ios26_or_later() {
            capabilities = vec![
                "minimal_mode",
                "ios26_beta_compatible",
                "idb_fallback",
                "limited_axp",
            ];
        }

        let mut result = serde_json::json!({
            "success": true,
            "harness_type": "axp",
            "bundle_path": bundle_path,
            "socket_path": socket_path.to_string_lossy(),
            "app_bundle_id": app_bundle_id,
            "capabilities": capabilities,
            "performance": {
                "expected_tap_time": if ios_version.is_ios26_or_later() {
                    "~100ms (iOS 26 beta fallback)"
                } else {
                    "<30ms"
                },
                "method": if ios_version.is_ios26_or_later() {
                    "IDB fallback"
                } else {
                    "AXP direct"
                }
            },
            "next_steps": [
                "Harness is installed and ready to use",
                "UI interactions will now use fast AXP taps automatically",
                "No additional setup needed - just use ui_interaction tool"
            ]
        });

        if ios_version.is_ios26_or_later() {
            result["ios26_beta_info"] = serde_json::json!({
                "detected": true,
                "runtime": ios_version.display_string(),
                "mode": "minimal",
                "note": "Using iOS 26 beta compatibility mode. AXP symbols may not be available, falling back to IDB for touch injection.",
                "recommendation": "For best performance, use iOS 18 simulators or wait for stable iOS 26 release"
            });
        }

        result
    }
}

#[async_trait]
impl Tool for AxpHarnessBuilder {
    async fn execute(&self, params: Value) -> Result<Value> {
        let app_bundle_id = params
            .get("app_bundle_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("app_bundle_id is required".to_string()))?;

        let harness_type = params
            .get("harness_type")
            .and_then(|v| v.as_str())
            .unwrap_or("axp");

        match harness_type {
            "axp" => self.build_axp_harness(app_bundle_id).await,
            _ => Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "UNSUPPORTED_HARNESS_TYPE",
                    "message": format!("Harness type '{}' is not supported", harness_type),
                    "supported_types": ["axp"]
                }
            })),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

impl Drop for AxpHarnessBuilder {
    fn drop(&mut self) {
        // Socket cleanup is handled by SocketManager's drop implementation
    }
}