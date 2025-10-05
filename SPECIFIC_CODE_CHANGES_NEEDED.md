# Specific Code Changes Needed for AXP Translator Support

## 1. Update `xctest_enhanced.rs` Launch Method

### Current Code (Line ~112):
```rust
// Run using xcrun simctl to launch the test runner
let child = Command::new("xcrun")
    .args([
        "simctl",
        "spawn",
        device_id,
        "xctest",
        bundle_path.to_str().unwrap(),
    ])
    .spawn()
```

### Change To:
```rust
// Run using xcodebuild test to get Apple's entitlements
let child = Command::new("xcodebuild")
    .args([
        "test",
        "-scheme", "ArkavoTestRunner",
        "-destination", &format!("id={}", device_id),
        "-only-testing:ArkavoTestRunnerUITests/ArkavoTestRunner/testRunServer",
    ])
    .spawn()
```

## 2. Update `ArkavoTestRunnerEnhanced.swift.template`

### Add Entitlement Check (Line ~50):
```swift
override func setUp() {
    super.setUp()
    
    // Verify we have AXP entitlements
    if !hasAXPEntitlements() {
        XCTFail("Missing required entitlements - not running in UI Test Runner")
    }
    
    socketServer = UnixSocketServer(path: socketPath)
    socketServer.delegate = self
    socketServer.start()
}

private func hasAXPEntitlements() -> Bool {
    // Check if we can access AXPTranslator
    let axpClass = NSClassFromString("AXPTranslator_iOS")
    return axpClass != nil
}
```

## 3. Update `idb_direct` to Use Existing Bridge

### In `lib.rs`, modify tap function:
```rust
pub fn tap(&self, x: f64, y: f64) -> Result<()> {
    // Check for XCTest harness first
    if std::path::Path::new("/tmp/arkavo_test.sock").exists() {
        return self.tap_via_xctest_bridge(x, y);
    }
    
    // Original implementation
    unsafe {
        let err = idb_tap(x, y);
        if err != IDB_SUCCESS {
            return Err(err.into());
        }
    }
    Ok(())
}

fn tap_via_xctest_bridge(&self, x: f64, y: f64) -> Result<()> {
    use crate::mcp::xctest_unix_bridge::{Command, CommandParameters, CommandType, TargetType};
    
    let cmd = Command {
        id: uuid::Uuid::new_v4().to_string(),
        command_type: CommandType::Tap,
        parameters: CommandParameters {
            target_type: Some(TargetType::Coordinate),
            x: Some(x),
            y: Some(y),
            ..Default::default()
        },
    };
    
    // Send via existing Unix bridge
    let bridge = XCTestUnixBridge::new();
    match bridge.send_command(cmd) {
        Ok(response) if response.success => Ok(()),
        Ok(response) => Err(IdbError::OperationFailed),
        Err(e) => Err(IdbError::OperationFailed),
    }
}
```

## 4. Create Xcode Project Structure

### Create `ios/ArkavoTestRunner/ArkavoTestRunner.xcodeproj`:
```bash
cd ios
mkdir -p ArkavoTestRunner/ArkavoTestRunnerUITests

# Copy template to actual test file
cp ../../crates/arkavo-test/templates/XCTestRunner/ArkavoTestRunnerEnhanced.swift.template \
   ArkavoTestRunner/ArkavoTestRunnerUITests/ArkavoTestRunner.swift

# Generate Xcode project
cd ArkavoTestRunner
swift package init --type library
swift package generate-xcodeproj

# Add UI test target manually in Xcode or via xcodeproj gem
```

## 5. Update CI to Launch Harness

### In `.github/workflows/test.yml`:
```yaml
- name: Start XCTest Harness
  if: matrix.os == 'macos-latest'
  run: |
    # Boot simulator
    xcrun simctl boot "iPhone 16 Pro Max" || true
    
    # Build and launch harness in background
    cd ios/ArkavoTestRunner
    xcodebuild test \
      -scheme ArkavoTestRunner \
      -destination 'platform=iOS Simulator,name=iPhone 16 Pro Max' \
      -only-testing:ArkavoTestRunnerUITests/ArkavoTestRunner/testRunServer &
    
    # Wait for socket
    timeout 30 bash -c 'until [ -S /tmp/arkavo_test.sock ]; do sleep 1; done'

- name: Run Tests with Harness
  run: |
    cargo test --features xctest-harness
```

## Summary of Changes:
1. ✅ Use existing `xctest_unix_bridge.rs` infrastructure
2. ✅ Launch via `xcodebuild test` instead of `simctl spawn`
3. ✅ Add entitlement verification
4. ✅ Route taps through existing socket bridge
5. ✅ Minimal new code required

The key insight: **We already built 90% of this!** We just need to launch it differently to get Apple's entitlements.