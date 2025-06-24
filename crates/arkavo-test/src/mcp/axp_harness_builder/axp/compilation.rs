use crate::{Result, TestError};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Handles compilation of AXP harness with various SDK configurations
pub struct HarnessCompiler;

impl HarnessCompiler {
    /// Compile harness using Swift Package Manager
    pub async fn compile_with_spm(
        build_dir: &Path,
        plist_path: &Path,
        sim_version: &str,
    ) -> Result<Value> {
        eprintln!("[AxpHarnessBuilder] Building with Swift Package Manager...");

        // Build using swift build
        let output = Command::new("swift")
            .args([
                "build",
                "-c",
                "release",
                "--arch",
                "arm64",
                "--package-path",
                build_dir.to_str().unwrap(),
            ])
            .env("SWIFT_PLATFORM", "iphonesimulator")
            .env("IPHONEOS_DEPLOYMENT_TARGET", "15.0")
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run swift build: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!(
                "[AxpHarnessBuilder] Swift PM build failed: {}",
                stderr.trim()
            );

            // Fall back to direct compilation
            return Self::compile_direct(build_dir, plist_path, sim_version).await;
        }

        // Find the built library
        let build_products = build_dir
            .join(".build")
            .join("arm64-apple-ios15.0-simulator")
            .join("release");

        let dylib_path = build_products.join("libArkavoHarness.dylib");
        if !dylib_path.exists() {
            return Err(TestError::Mcp(
                "Built library not found after successful build".to_string(),
            ));
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
            "bundle_path": xctest_bundle.to_string_lossy().to_string(),
            "compilation_method": "swift_pm"
        }))
    }

    /// Direct compilation using swiftc
    pub async fn compile_direct(
        build_dir: &Path,
        plist_path: &Path,
        sim_version: &str,
    ) -> Result<Value> {
        eprintln!(
            "[AxpHarnessBuilder] Falling back to direct swiftc compilation for iOS {}...",
            sim_version
        );

        // Determine if this is iOS 26 beta
        let is_ios26_beta = sim_version == "26";

        // List available SDKs
        let sdk_list_output = Command::new("xcrun")
            .args(["simctl", "list", "runtimes", "--json"])
            .output()
            .ok();

        if let Some(output) = sdk_list_output {
            if output.status.success() {
                eprintln!("[AxpHarnessBuilder] Available runtimes:");
                eprintln!("{}", String::from_utf8_lossy(&output.stdout));
            }
        }

        // Get SDK path
        let sdk_output = Command::new("xcrun")
            .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute xcrun: {}", e)))?;

        if !sdk_output.status.success() {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": super::constants::error_codes::SDK_NOT_FOUND,
                    "message": "Failed to get iOS simulator SDK path",
                    "details": String::from_utf8_lossy(&sdk_output.stderr).to_string()
                }
            }));
        }

        let sdk_path = String::from_utf8_lossy(&sdk_output.stdout).trim().to_string();
        eprintln!("[AxpHarnessBuilder] Using SDK: {}", sdk_path);

        let swift_files = vec![
            build_dir.join("Sources/ArkavoHarness/ArkavoAXBridge.swift"),
            build_dir.join("Sources/ArkavoHarness/ArkavoTestRunner.swift"),
        ];

        let output_binary = build_dir.join("ArkavoHarness");

        let mut cmd = Command::new("swiftc");
        cmd.args([
            "-sdk",
            &sdk_path,
            "-target",
            &format!("arm64-apple-ios{}.0-simulator", sim_version),
            "-parse-as-library",
            "-emit-library",
            "-module-name",
            "ArkavoHarness",
            "-o",
            output_binary.to_str().unwrap(),
            "-suppress-warnings",
            "-framework",
            "Foundation",
            "-framework",
            "CoreGraphics",
        ]);

        // Add XCTest framework only if not iOS 26 beta
        if !is_ios26_beta {
            cmd.args([
                "-framework",
                "XCTest",
                "-F",
                &format!("{}/System/Library/Frameworks", sdk_path),
                "-F",
                &format!("{}/../../Library/Frameworks", sdk_path),
                "-F",
                &format!(
                    "{}/../../../../Platforms/iPhoneOS.platform/Library/Developer/CoreSimulator/Frameworks",
                    sdk_path
                ),
                "-F",
                &format!("{}/System/Library/PrivateFrameworks", sdk_path),
                "-L",
                &format!("{}/usr/lib", sdk_path),
                "-L",
                &format!("{}/usr/lib/swift", sdk_path),
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                "@executable_path/Frameworks",
                "-Xlinker",
                "-rpath",
                "-Xlinker",
                "@loader_path/Frameworks",
            ]);
        } else {
            eprintln!("[AxpHarnessBuilder] iOS 26 beta detected - skipping XCTest framework");
            cmd.args(["-D", "IOS_26_BETA"]);
        }

        // Add Swift files
        for file in &swift_files {
            cmd.arg(file.to_str().unwrap());
        }

        eprintln!("[AxpHarnessBuilder] Running: {:?}", cmd);

        let output = cmd
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to compile: {}", e)))?;

        if !output.status.success() {
            let error_details = String::from_utf8_lossy(&output.stderr).to_string();

            // Check for iOS 26 specific errors
            if is_ios26_beta
                && (error_details.contains("XCTest")
                    || error_details.contains("undefined symbol")
                    || error_details.contains("cannot find"))
            {
                eprintln!(
                    "[AxpHarnessBuilder] iOS 26 beta compilation failed with expected errors"
                );
                return Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": super::constants::error_codes::BETA_COMPILATION_FAILED,
                        "message": "iOS 26 beta compilation failed - XCTest symbols not available",
                        "recommendation": "Use IDB or standard UI automation instead of AXP",
                        "details": error_details,
                        "fallback_available": true
                    }
                }));
            }

            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": super::constants::error_codes::COMPILATION_FAILED,
                    "message": "Failed to compile AXP harness",
                    "details": error_details,
                    "sdk_path": sdk_path,
                    "target_ios": sim_version,
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
            "bundle_path": xctest_bundle.to_string_lossy().to_string(),
            "compilation_method": "direct_swiftc",
            "ios_version": sim_version,
            "beta_mode": is_ios26_beta
        }))
    }
}