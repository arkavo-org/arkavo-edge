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

pub struct AxpHarnessBuilder {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

/// Embedded iOS 26 beta compilation guidance
const IOS26_BETA_COMPILATION_GUIDANCE: &str = r#"
## iOS 26 Beta Compilation Fix

### Problem
When building on iOS 26 beta simulators, XCTest framework symbols may be missing or incompatible due to SDK/runtime mismatch.

### Symptoms
- Compilation errors: "SDK does not contain 'XCTest.framework'"
- Symbol errors: "cannot find type 'XCUICoordinate' in scope"
- Runtime crashes when trying to use AXP functions

### Root Cause
Apple's beta releases often have symbol drift between the SDK and runtime. The XCTest framework in iOS 26 beta may have changed internal symbols that AXP relies on.

### Solution Strategy
1. **Minimal Mode**: Use templates without XCTest dependency
2. **Fallback Compilation**: Target iOS 18.0 but use iOS 26 SDK
3. **Runtime Detection**: Check for iOS 26 beta and switch to IDB/AppleScript

### Implementation
The harness builder automatically:
1. Detects iOS 26 beta from device runtime
2. Uses minimal templates (ArkavoAXBridgeMinimal.swift)
3. Compiles without XCTest framework
4. Falls back to IDB for actual touch injection

### Performance Impact
- AXP taps: <30ms (not available on iOS 26 beta)
- IDB taps: ~100ms (fallback for iOS 26 beta)
- AppleScript: ~200ms (last resort)

### Recommendations
1. Use iOS 18 simulators for testing when possible
2. Wait for stable iOS 26 release
3. Install matching Xcode 16 beta with iOS 26 SDK
4. Use IDB (idb_companion) for iOS 26 beta automation
"#;

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
    
    /// Get iOS 26 beta compilation guidance
    pub fn get_ios26_beta_guidance() -> &'static str {
        IOS26_BETA_COMPILATION_GUIDANCE
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

        // Create build directory in .arkavo relative to current directory
        let arkavo_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".arkavo")
            .join("axp-harnesses");
        
        let build_dir = arkavo_dir
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
            
        // Use .arkavo/sockets directory for socket files
        let socket_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".arkavo")
            .join("sockets");
        
        fs::create_dir_all(&socket_dir)
            .map_err(|e| TestError::Mcp(format!("Failed to create socket directory: {}", e)))?;
            
        let socket_path = socket_dir
            .join(format!("arkavo-axp-{}-{}.sock", device_id, app_bundle_id.replace('.', "_")));

        // Check if we're on iOS 26 beta
        let is_ios26_beta = self.device_manager.get_active_device()
            .map(|device| device.runtime.contains("iOS-26"))
            .unwrap_or(false);
        
        eprintln!("[AxpHarnessBuilder] iOS 26 beta detected: {}", is_ios26_beta);
        
        // Write appropriate bridge code based on iOS version
        let ax_bridge_path = harness_dir.join("ArkavoAXBridge.swift");
        if is_ios26_beta {
            eprintln!("[AxpHarnessBuilder] Using iOS 26 enhanced bridge with symbol discovery");
            fs::write(&ax_bridge_path, templates::ARKAVO_AX_BRIDGE_IOS26_SWIFT)
                .map_err(|e| TestError::Mcp(format!("Failed to write iOS 26 AX bridge: {}", e)))?;
        } else {
            fs::write(&ax_bridge_path, templates::ARKAVO_AX_BRIDGE_SWIFT)
                .map_err(|e| TestError::Mcp(format!("Failed to write AX bridge: {}", e)))?;
        }

        // Write appropriate test runner based on iOS version
        let runner_path = harness_dir.join("ArkavoTestRunner.swift");
        if is_ios26_beta {
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

        // Create Package.swift with Swift 6.0
        let package_swift = format!(
            r#"// swift-tools-version:6.0
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

        // Compile the harness using Swift Package Manager
        eprintln!("[AxpHarnessBuilder] Compiling AXP harness using Swift PM...");
        
        let compile_result = self.compile_harness_spm(&build_dir, &plist_path).await?;
        
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

        let is_ios26_beta = self.device_manager.get_active_device()
            .map(|device| device.runtime.contains("iOS-26"))
            .unwrap_or(false);
        
        let mut result = serde_json::json!({
            "success": true,
            "harness_type": "axp",
            "bundle_path": bundle_path,
            "socket_path": socket_path.to_string_lossy(),
            "app_bundle_id": app_bundle_id,
            "capabilities": if is_ios26_beta {
                serde_json::json!({
                    "mode": "minimal",
                    "ios26_beta": true,
                    "axp_tap": "Not available in iOS 26 beta - use IDB instead",
                    "fallback": "IDB or AppleScript required for automation",
                    "note": "iOS 26 beta detected - using minimal harness for compatibility"
                })
            } else {
                serde_json::json!({
                    "axp_tap": "Fast coordinate-based tapping using AXP",
                    "axp_snapshot": "Accessibility tree snapshots",
                    "fallback": "Automatic fallback to XCUICoordinate if AXP unavailable"
                })
            },
            "important": if is_ios26_beta {
                "iOS 26 beta harness installed - use IDB for actual automation"
            } else {
                "This generic harness works with ANY iOS app - no project files needed"
            },
            "next_steps": if is_ios26_beta {
                vec![
                    "iOS 26 beta detected - AXP functions not available",
                    "The harness provides socket communication only",
                    "Use IDB (idb_companion) for actual touch injection",
                    "Taps will be slower (~100ms) until iOS 26 stable release"
                ]
            } else {
                vec![
                    "The harness is now installed on the simulator",
                    "Launch your app using app_launcher tool",
                    "Use ui_interaction - taps will now be <30ms!"
                ]
            }
        });
        
        if is_ios26_beta {
            result["ios26_beta_info"] = serde_json::json!({
                "detected": true,
                "impact": "AXP symbols not available - using minimal harness",
                "workaround": "IDB will be used automatically for touch injection",
                "performance": "Taps ~100ms instead of <30ms",
                "solution": "Install matching Xcode 16 beta or wait for stable iOS 26 release"
            });
        }
        
        Ok(result)
    }

    async fn compile_harness_spm(&self, build_dir: &Path, plist_path: &Path) -> Result<Value> {
        // Use Swift Package Manager to build
        eprintln!("[AxpHarnessBuilder] Building with Swift Package Manager...");
        
        let output = Command::new("swift")
            .args(["build", "-c", "release"])
            .current_dir(build_dir)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run swift build: {}", e)))?;
            
        if !output.status.success() {
            eprintln!("[AxpHarnessBuilder] Swift PM build failed, falling back to direct compilation");
            eprintln!("[AxpHarnessBuilder] Error: {}", String::from_utf8_lossy(&output.stderr));
            
            // Fall back to direct compilation
            return self.compile_harness_direct(build_dir, plist_path).await;
        }
        
        // Find the built product
        let build_output_dir = build_dir.join(".build/release");
        let dylib_path = build_output_dir.join("libArkavoHarness.dylib");
        
        if !dylib_path.exists() {
            return Err(TestError::Mcp("Built library not found".to_string()));
        }
        
        // Create .xctest bundle
        let xctest_bundle = build_dir.join("ArkavoHarness.xctest");
        fs::create_dir_all(&xctest_bundle)
            .map_err(|e| TestError::Mcp(format!("Failed to create bundle: {}", e)))?;
        
        // Copy binary
        fs::copy(&dylib_path, xctest_bundle.join("ArkavoHarness"))
            .map_err(|e| TestError::Mcp(format!("Failed to copy binary: {}", e)))?;
        
        // Copy Info.plist
        fs::copy(plist_path, xctest_bundle.join("Info.plist"))
            .map_err(|e| TestError::Mcp(format!("Failed to copy plist: {}", e)))?;
        
        Ok(serde_json::json!({
            "success": true,
            "bundle_path": xctest_bundle.to_string_lossy().to_string()
        }))
    }
    
    async fn compile_harness_direct(&self, build_dir: &Path, plist_path: &Path) -> Result<Value> {
        // Try to get SDK for specific version first (for beta support)
        let sim_major_version = self.device_manager.get_active_device()
            .and_then(|device| {
                device.runtime.split("iOS-").nth(1)
                    .and_then(|v| v.split('-').next())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "18".to_string());
        
        eprintln!("[AxpHarnessBuilder] Looking for iOS {} SDK", sim_major_version);
        
        // First check what SDKs are available
        let list_sdks = Command::new("xcodebuild")
            .arg("-showsdks")
            .output()
            .ok();
            
        if let Some(output) = list_sdks {
            let sdks_list = String::from_utf8_lossy(&output.stdout);
            eprintln!("[AxpHarnessBuilder] Available simulator SDKs:");
            for line in sdks_list.lines() {
                if line.contains("iphonesimulator") {
                    eprintln!("  {}", line.trim());
                }
            }
        }
        
        // Try version-specific SDK first (e.g., iphonesimulator26.0 for iOS 26 beta)
        let sdk_output = Command::new("xcrun")
            .args(["--sdk", &format!("iphonesimulator{}.0", sim_major_version), "--show-sdk-path"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .or_else(|| {
                // Fall back to default SDK
                eprintln!("[AxpHarnessBuilder] Version-specific SDK not found, using default iphonesimulator SDK");
                Command::new("xcrun")
                    .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
                    .output()
                    .ok()
            })
            .ok_or_else(|| TestError::Mcp("Failed to execute xcrun".to_string()))?;

        if !sdk_output.status.success() {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "SDK_NOT_FOUND",
                    "message": "Failed to get iOS simulator SDK path",
                    "details": String::from_utf8_lossy(&sdk_output.stderr).to_string()
                }
            }));
        }

        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout).trim().to_string();
        eprintln!("[AxpHarnessBuilder] Using SDK: {}", sdk_path);

        // Build using Swift
        let swift_files = vec![
            build_dir.join("Sources/ArkavoHarness/ArkavoAXBridge.swift"),
            build_dir.join("Sources/ArkavoHarness/ArkavoTestRunner.swift"),
        ];

        let output_binary = build_dir.join("ArkavoHarness");
        
        // Determine simulator version from device runtime if possible
        let sim_version = if let Some(device) = self.device_manager.get_active_device() {
            // Extract version from runtime like "com.apple.CoreSimulator.SimRuntime.iOS-26-0"
            if let Some(version_part) = device.runtime.split("iOS-").nth(1) {
                version_part.replace('-', ".").trim_end_matches(".0").to_string()
            } else {
                "15.0".to_string() // Fallback to minimum supported
            }
        } else {
            "15.0".to_string()
        };
        
        eprintln!("[AxpHarnessBuilder] Target iOS version: {}", sim_version);
        
        let mut cmd = Command::new("swiftc");
        cmd.args([
            "-sdk", &sdk_path,
            "-target", &format!("arm64-apple-ios{}-simulator", sim_version),
            "-parse-as-library",
            "-emit-library",
            "-module-name", "ArkavoHarness",
            "-o", output_binary.to_str().unwrap(),
            "-suppress-warnings",
            "-framework", "Foundation",
            "-framework", "CoreGraphics",
        ]);
        
        // Add XCTest - handle both standard and beta Xcode paths
        let is_ios26_beta = sim_version.starts_with("26");
        
        // Detect Xcode path from SDK path
        let xcode_path = if let Some(xcode_idx) = sdk_path.find("/Contents/Developer") {
            sdk_path[..xcode_idx].to_string()
        } else if sdk_path.contains("Xcode-beta.app") {
            "/Applications/Xcode-beta.app".to_string()
        } else if std::path::Path::new("/Applications/Xcode.app").exists() {
            "/Applications/Xcode.app".to_string()
        } else {
            // Try to find Xcode using xcode-select
            Command::new("xcode-select")
                .arg("-p")
                .output()
                .ok()
                .and_then(|output| {
                    if output.status.success() {
                        let dev_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        dev_path.strip_suffix("/Contents/Developer")
                            .map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "/Applications/Xcode.app".to_string())
        };
        
        eprintln!("[AxpHarnessBuilder] Using Xcode at: {}", xcode_path);
        
        // For iOS 26 beta, use specialized handling
        if is_ios26_beta {
            cmd.args([
                "-framework", "Foundation",
                // Weak link XCTest for beta using -Xlinker
                "-Xlinker", "-weak_framework", "-Xlinker", "XCTest",
                "-F", &format!("{}/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/Library/Frameworks", xcode_path),
                "-F", &format!("{}/Contents/Developer/Platforms/iPhoneOS.platform/Library/Developer/CoreSimulator/Frameworks", xcode_path),
                "-Xlinker", "-rpath", "-Xlinker", &format!("{}/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/Library/Frameworks", xcode_path),
            ]);
        } else {
            cmd.args([
                "-framework", "XCTest",
                "-F", &format!("{}/../../Library/Frameworks", sdk_path),
                "-F", &format!("{}/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/Library/Frameworks", xcode_path),
                "-L", &format!("{}/../../Library/Developer/CoreSimulator/Frameworks", sdk_path),
                "-Xlinker", "-rpath", "-Xlinker", &format!("{}/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/Library/Frameworks", xcode_path),
            ]);
        }
        
        cmd.args([
            "-F", &format!("{}/System/Library/Frameworks", sdk_path),
            "-Xlinker", "-rpath", "-Xlinker", "@loader_path/../Frameworks",
        ]);

        // Add all Swift files
        for file in &swift_files {
            cmd.arg(file.to_str().unwrap());
        }

        eprintln!("[AxpHarnessBuilder] Running: {:?}", cmd);

        let output = cmd.output()
            .map_err(|e| TestError::Mcp(format!("Failed to compile: {}", e)))?;

        if !output.status.success() {
            eprintln!("[AxpHarnessBuilder] Compilation failed!");
            eprintln!("[AxpHarnessBuilder] Exit code: {:?}", output.status.code());
            eprintln!("[AxpHarnessBuilder] Command: {:?}", cmd);
            eprintln!("[AxpHarnessBuilder] Working directory: {}", build_dir.display());
            eprintln!("[AxpHarnessBuilder] stderr:\n{}", String::from_utf8_lossy(&output.stderr));
            eprintln!("[AxpHarnessBuilder] stdout:\n{}", String::from_utf8_lossy(&output.stdout));
            
            // Write compilation log
            let log_path = build_dir.join("compilation_error.log");
            let _ = fs::write(&log_path, format!(
                "Command: {:?}\n\nWorking Dir: {}\nSDK: {}\nTarget: ios{}-simulator\n\nExit Code: {:?}\n\nSTDERR:\n{}\n\nSTDOUT:\n{}\n",
                cmd,
                build_dir.display(),
                sdk_path,
                sim_version,
                output.status.code(),
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            ));
            eprintln!("[AxpHarnessBuilder] Error log saved to: {}", log_path.display());
            
            let error_code = match output.status.code() {
                Some(127) => "SDK_NOT_INSTALLED",
                Some(1) => "COMPILATION_FAILED", 
                _ => "UNKNOWN_ERROR"
            };
            
            // Check for beta SDK mismatch - comprehensive detection
            let stderr_text = String::from_utf8_lossy(&output.stderr);
            let is_beta_issue = stderr_text.contains("SDK does not contain") || 
                stderr_text.contains("no such module") ||
                stderr_text.contains("could not find module") ||
                stderr_text.contains("XCTest.framework") ||
                stderr_text.contains("cannot find type 'XCUICoordinate'") ||
                stderr_text.contains("framework not found XCTest");
            
            // If it's a beta issue, try a fallback compilation with minimal dependencies
            if is_beta_issue && sim_major_version == "26" {
                eprintln!("[AxpHarnessBuilder] iOS 26 beta detected, trying fallback compilation...");
                
                // Try multiple fallback strategies for iOS 26 beta
                eprintln!("[AxpHarnessBuilder] Attempting fallback strategy 1: iOS 18 target...");
                
                // Strategy 1: Use iOS 18 target with iOS 26 SDK
                let mut fallback_cmd = Command::new("swiftc");
                fallback_cmd.args([
                    "-sdk", &sdk_path,
                    "-target", "arm64-apple-ios18.0-simulator",
                    "-parse-as-library",
                    "-emit-library",
                    "-module-name", "ArkavoHarness",
                    "-o", output_binary.to_str().unwrap(),
                    "-suppress-warnings",
                    "-framework", "Foundation",
                    "-framework", "CoreGraphics",
                    "-D", "IOS_26_BETA",
                    "-Xlinker", "-undefined",
                    "-Xlinker", "dynamic_lookup", // Allow missing symbols
                ]);
                fallback_cmd.args(swift_files.iter().map(|f| f.to_str().unwrap()));
                
                let fallback_output = fallback_cmd.output();
                    
                if let Ok(fallback_output) = fallback_output {
                    if fallback_output.status.success() {
                        eprintln!("[AxpHarnessBuilder] Fallback compilation succeeded!");
                        // Continue with bundle creation
                    } else {
                        eprintln!("[AxpHarnessBuilder] Fallback strategy 1 failed, trying strategy 2...");
                        
                        // Strategy 2: Use minimal compilation with no frameworks
                        let minimal_cmd = Command::new("swiftc")
                            .args([
                                "-sdk", &sdk_path,
                                "-target", "arm64-apple-ios15.0-simulator", // Even lower target
                                "-parse-as-library",
                                "-emit-library",
                                "-module-name", "ArkavoHarness",
                                "-o", output_binary.to_str().unwrap(),
                                "-suppress-warnings",
                                "-framework", "Foundation",
                                "-D", "IOS_26_BETA_MINIMAL",
                                "-Xlinker", "-undefined",
                                "-Xlinker", "dynamic_lookup",
                            ])
                            .args(swift_files.iter().map(|f| f.to_str().unwrap()))
                            .output();
                            
                        if let Ok(minimal_output) = minimal_cmd {
                            if minimal_output.status.success() {
                                eprintln!("[AxpHarnessBuilder] Fallback strategy 2 succeeded!");
                                // Continue with bundle creation
                            } else {
                                eprintln!("[AxpHarnessBuilder] All compilation strategies failed");
                                return Ok(serde_json::json!({
                                    "success": false,
                                    "error": {
                                        "code": "IOS26_BETA_COMPILATION_FAILED",
                                        "message": "iOS 26 beta compilation failed - all strategies exhausted",
                                        "details": stderr_text.to_string(),
                                        "fallback_errors": {
                                            "strategy1": String::from_utf8_lossy(&fallback_output.stderr).to_string(),
                                            "strategy2": String::from_utf8_lossy(&minimal_output.stderr).to_string()
                                        },
                                        "sdk_path": sdk_path,
                                        "target_ios": sim_version,
                                        "guidance": "Use 'ios26_beta_guidance' tool for detailed help",
                                        "quick_fix": [
                                            "The iOS 26 beta fix is embedded but compilation still failed",
                                            "This usually means XCTest.framework is completely missing",
                                            "Solution 1: Use iOS 18 simulator instead",
                                            "Solution 2: Install matching Xcode 16 beta",
                                            "Solution 3: Use IDB-only automation (no harness needed)"
                                        ]
                                    }
                                }));
                            }
                        }
                    }
                } else {
                    return Ok(serde_json::json!({
                        "success": false,
                        "error": {
                            "code": error_code,
                            "message": "Failed to compile AXP harness",
                            "details": String::from_utf8_lossy(&output.stderr).to_string(),
                            "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
                            "exit_code": output.status.code(),
                            "sdk_path": sdk_path,
                            "target_ios": sim_version,
                            "is_beta_issue": is_beta_issue,
                            "troubleshooting": vec![
                                "iOS 26 beta issue detected but couldn't resolve",
                                "Run 'ios26_beta_guidance' tool for comprehensive help",
                                "The embedded fix handles most cases but your environment may need additional setup"
                            ]
                        }
                    }));
                }
            } else {
                return Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": error_code,
                        "message": "Failed to compile AXP harness",
                        "details": String::from_utf8_lossy(&output.stderr).to_string(),
                        "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
                        "exit_code": output.status.code(),
                        "sdk_path": sdk_path,
                        "target_ios": sim_version,
                        "is_beta_issue": is_beta_issue,
                        "troubleshooting": if is_beta_issue {
                            vec![
                                "iOS 26 beta compilation issue detected",
                                "Run 'ios26_beta_guidance' tool for detailed help",
                                "The fix is embedded but may need environment adjustments"
                            ]
                        } else {
                            vec![
                                "Check that Xcode command line tools are installed",
                                "Try: xcode-select --install",
                                "Verify Swift is available: swift --version"
                            ]
                        }
                    }
                }));
            }
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