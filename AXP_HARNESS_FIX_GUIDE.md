# AXP Harness iOS 26 Beta Compilation Fix Guide

## Problem Statement
The AXP harness fails to compile on iOS 26 beta due to SDK changes and symbol drift. This guide provides the exact code changes needed to fix the compilation.

## Implementation Steps

### Step 1: Update axp_harness_builder.rs

Replace the `compile_harness` method starting at line 219 with:

```rust
async fn compile_harness(&self, build_dir: &Path, plist_path: &Path) -> Result<Value> {
    // List available SDKs first
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

    // Try to get SDK for specific version first (for beta support)
    let sim_major_version = self.device_manager.get_active_device()
        .and_then(|device| {
            device.runtime.split("iOS-").nth(1)
                .and_then(|v| v.split('-').next())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "18".to_string());
    
    // Try version-specific SDK first
    let sdk_output = Command::new("xcrun")
        .args(["--sdk", &format!("iphonesimulator{}.0", sim_major_version), "--show-sdk-path"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .or_else(|| {
            eprintln!("[AxpHarnessBuilder] Beta SDK not found, using default iphonesimulator SDK");
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

    // Compile with enhanced framework paths
    let compile_result = self.compile_with_sdk(&sdk_path, build_dir, &sim_major_version).await;
    
    // If beta compilation fails, try fallback
    if !compile_result["success"].as_bool().unwrap_or(false) && sim_major_version == "26" {
        eprintln!("[AxpHarnessBuilder] iOS 26 beta compilation failed, trying fallback approach");
        return self.compile_with_fallback(build_dir, &sdk_path).await;
    }
    
    compile_result
}

async fn compile_with_sdk(&self, sdk_path: &str, build_dir: &Path, sim_version: &str) -> Result<Value> {
    let swift_files = vec![
        build_dir.join("Sources/ArkavoHarness/ArkavoAXBridge.swift"),
        build_dir.join("Sources/ArkavoHarness/ArkavoTestRunner.swift"),
    ];

    let output_binary = build_dir.join("ArkavoHarness");
    
    let mut cmd = Command::new("swiftc");
    cmd.args([
        "-sdk", sdk_path,
        "-target", &format!("arm64-apple-ios{}-simulator", sim_version),
        "-parse-as-library",
        "-emit-library",
        "-module-name", "ArkavoHarness",
        "-o", output_binary.to_str().unwrap(),
        "-suppress-warnings",
        "-framework", "Foundation",
        "-framework", "CoreGraphics",
        "-framework", "XCTest",
        "-F", &format!("{}/System/Library/Frameworks", sdk_path),
        "-F", &format!("{}/../../Library/Frameworks", sdk_path),
        "-F", &format!("{}/../../../../Platforms/iPhoneOS.platform/Library/Developer/CoreSimulator/Frameworks", sdk_path),
        "-F", &format!("{}/System/Library/PrivateFrameworks", sdk_path),
        "-L", &format!("{}/usr/lib", sdk_path),
        "-L", &format!("{}/usr/lib/swift", sdk_path),
        "-Xlinker", "-rpath", "-Xlinker", "@executable_path/Frameworks",
        "-Xlinker", "-rpath", "-Xlinker", "@loader_path/Frameworks",
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
                "details": String::from_utf8_lossy(&output.stderr).to_string(),
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
        "bundle_path": xctest_bundle.to_string_lossy().to_string()
    }))
}

async fn compile_with_fallback(&self, build_dir: &Path, sdk_path: &str) -> Result<Value> {
    eprintln!("[AxpHarnessBuilder] Attempting fallback compilation without XCTest");
    
    let swift_files = vec![
        build_dir.join("Sources/ArkavoHarness/ArkavoAXBridge.swift"),
        build_dir.join("Sources/ArkavoHarness/ArkavoTestRunner.swift"),
    ];

    let output_binary = build_dir.join("ArkavoHarness");
    
    let mut cmd = Command::new("swiftc");
    cmd.args([
        "-sdk", sdk_path,
        "-target", "arm64-apple-ios18.0-simulator", // Use iOS 18 target with iOS 26 SDK
        "-parse-as-library",
        "-emit-library",
        "-module-name", "ArkavoHarness",
        "-o", output_binary.to_str().unwrap(),
        "-suppress-warnings",
        "-framework", "Foundation",
        "-framework", "CoreGraphics",
        "-D", "IOS_26_BETA",
        "-F", &format!("{}/System/Library/Frameworks", sdk_path),
    ]);

    // Add all Swift files
    for file in &swift_files {
        cmd.arg(file.to_str().unwrap());
    }

    let output = cmd.output()
        .map_err(|e| TestError::Mcp(format!("Failed to compile fallback: {}", e)))?;

    if output.status.success() {
        // Create .xctest bundle
        let xctest_bundle = build_dir.join("ArkavoHarness.xctest");
        fs::create_dir_all(&xctest_bundle)?;
        fs::copy(&output_binary, xctest_bundle.join("ArkavoHarness"))?;
        fs::copy(build_dir.join("Info.plist"), xctest_bundle.join("Info.plist"))?;

        Ok(serde_json::json!({
            "success": true,
            "bundle_path": xctest_bundle.to_string_lossy().to_string(),
            "warning": "Compiled with iOS 18 target due to iOS 26 beta issues. Some features may be limited."
        }))
    } else {
        Ok(serde_json::json!({
            "success": false,
            "error": {
                "code": "BETA_COMPILATION_FAILED",
                "message": "Unable to compile for iOS 26 beta",
                "recommendation": "Use IDB or standard UI automation instead of AXP",
                "details": String::from_utf8_lossy(&output.stderr).to_string()
            }
        }))
    }
}
```

### Step 2: Update ArkavoAXBridge.swift

Add these conditional compilation checks at the top of the file:

```swift
import Foundation
#if canImport(XCTest)
import XCTest
#endif
#if canImport(UIKit)
import UIKit
#endif
#if canImport(CoreGraphics)
import CoreGraphics
#endif
```

Then wrap XCTest-dependent code:

```swift
/// Capture accessibility snapshot
@objc public func snapshot() -> Data? {
    #if canImport(XCTest)
    // For now, use XCUIScreen for screenshots
    let screenshot = XCUIScreen.main.screenshot()
    return screenshot.pngRepresentation
    #else
    // Return nil when XCTest is not available
    print("[ArkavoAXBridge] XCTest not available for screenshots")
    return nil
    #endif
}

/// Fallback tap using XCUICoordinate (when AXP unavailable)
@objc public func fallbackTap(x: Double, y: Double) -> Bool {
    #if canImport(XCTest)
    let app = XCUIApplication()
    let coordinate = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
        .withOffset(CGVector(dx: x, dy: y))
    coordinate.tap()
    return true
    #else
    print("[ArkavoAXBridge] XCTest not available for fallback tap")
    return false
    #endif
}
```

### Step 3: Test Commands

After implementing the changes, test with:

```bash
# Build arkavo-edge
cargo build

# Test the AXP harness builder
./target/debug/arkavo-test-mcp

# In another terminal, send a test request:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"build_test_harness","arguments":{"app_bundle_id":"com.example.testapp"}}}' | nc localhost 8080
```

### Step 4: Verification Script

Create this diagnostic script at `/tmp/verify_axp_fix.sh`:

```bash
#!/bin/bash
echo "=== Verifying AXP Harness Fix ==="

# Check Xcode version
echo -e "\n1. Xcode Version:"
xcodebuild -version

# List available SDKs
echo -e "\n2. Available SDKs:"
xcodebuild -showsdks | grep -E "Simulator|simruntime"

# Check specific SDK paths
echo -e "\n3. SDK Paths:"
xcrun --sdk iphonesimulator --show-sdk-path
xcrun --sdk iphonesimulator26.0 --show-sdk-path 2>/dev/null || echo "iOS 26 SDK not found"

# Test Swift compilation
echo -e "\n4. Testing Swift compilation:"
cat > /tmp/test_axp.swift << 'EOF'
import Foundation
#if canImport(XCTest)
import XCTest
print("XCTest available")
#else
print("XCTest not available")
#endif
EOF

swiftc -sdk $(xcrun --sdk iphonesimulator --show-sdk-path) \
       -target arm64-apple-ios18.0-simulator \
       -parse-as-library \
       /tmp/test_axp.swift \
       -o /tmp/test_axp 2>&1

echo -e "\n5. Compilation result: $?"
```

## Expected Outcomes

1. **Primary Path**: Compilation succeeds with iOS 26 beta SDK
2. **Fallback Path**: If iOS 26 fails, compiles with iOS 18 target
3. **Error Handling**: Clear messages guide users to alternatives

## Notes for Testing Agent

- Always run `build_test_harness` before UI automation
- If AXP fails, the system automatically uses slower fallback methods
- Monitor socket creation at `/tmp/arkavo-axp-*.sock`
- Check compilation logs for framework loading issues