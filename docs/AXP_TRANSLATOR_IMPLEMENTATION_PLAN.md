# AXP Translator Implementation Plan for Arkavo Edge

## Overview

To use the private AXPTranslator APIs on Xcode 16+, we need to leverage Apple's signed UI Test Runner app that carries the necessary entitlements.

## Key Insights

1. **Apple-signed runner**: The UI Test Runner.app inside Xcode is signed by Apple with private entitlements
2. **Entitlements**: It includes `com.apple.private.axplatformtranslator` and `com.apple.accessibility.axuielement`
3. **Process inheritance**: Our test code runs inside this entitled process and inherits its privileges
4. **Simulator-only**: This approach only works in simulators, not on real devices

## Implementation Steps

### 1. Create UI Test Target

Create a minimal UI test target that will be run by Apple's UI Test Runner:

```swift
// ArkavoUITapHarness.swift
import XCTest

class ArkavoUITapHarness: XCTestCase {
    var socketServer: UnixSocketServer?
    
    override func setUp() {
        super.setUp()
        // Start Unix socket server at /tmp/arkavo_tap.sock
        socketServer = UnixSocketServer(path: "/tmp/arkavo_tap.sock")
        socketServer?.onCommand = { [weak self] command in
            self?.handleCommand(command)
        }
        socketServer?.start()
    }
    
    func testRunHarness() {
        // Keep test running to serve tap requests
        let expectation = self.expectation(description: "Keep alive")
        expectation.isInverted = true
        wait(for: [expectation], timeout: 3600) // 1 hour
    }
    
    func handleCommand(_ command: TapCommand) {
        switch command.type {
        case .tap:
            // Use XCTest's tap which internally uses AXPTranslator
            let app = XCUIApplication()
            let coordinate = app.coordinate(withNormalizedOffset: 
                CGVector(dx: command.x / app.frame.width, 
                        dy: command.y / app.frame.height))
            coordinate.tap()
        case .swipe:
            // Handle swipe
        }
    }
}
```

### 2. Modify arkavo-edge CLI

Update the IDB Direct implementation to check for the harness socket:

```rust
// In arkavo-idb-direct/src/lib.rs
impl IdbDirect {
    pub fn tap(&self, x: f64, y: f64) -> Result<()> {
        // Check if harness is available
        if Path::new("/tmp/arkavo_tap.sock").exists() {
            // Send tap via harness (which uses AXPTranslator)
            self.send_to_harness(TapCommand { x, y })?;
        } else {
            // Fall back to current implementation
            self.tap_via_hid(x, y)?;
        }
        Ok(())
    }
    
    fn send_to_harness(&self, command: TapCommand) -> Result<()> {
        // Connect to Unix socket and send command
        let mut stream = UnixStream::connect("/tmp/arkavo_tap.sock")?;
        let json = serde_json::to_string(&command)?;
        stream.write_all(json.as_bytes())?;
        Ok(())
    }
}
```

### 3. Launch Script

Create a script to launch the harness:

```bash
#!/bin/bash
# launch_tap_harness.sh

# Build and run the UI test harness
xcodebuild test \
    -workspace ArkavoUITapHarness.xcworkspace \
    -scheme ArkavoUITapHarness \
    -destination "platform=iOS Simulator,name=$1" \
    -only-testing:ArkavoUITapHarness/ArkavoUITapHarness/testRunHarness &

# Wait for socket to be available
while [ ! -S /tmp/arkavo_tap.sock ]; do
    sleep 0.1
done

echo "Tap harness ready at /tmp/arkavo_tap.sock"
```

### 4. Integration with Existing Test Infrastructure

Modify `XCTestEnhanced` to use this approach:

```rust
// In xctest_enhanced.rs
impl XCTestEnhanced {
    pub async fn initialize_with_harness(&self, device_id: &str) -> Result<()> {
        // Launch the UI test harness instead of direct test bundle
        let output = Command::new("./launch_tap_harness.sh")
            .arg(device_id)
            .spawn()?;
        
        // Wait for harness to be ready
        for _ in 0..30 {
            if Path::new("/tmp/arkavo_tap.sock").exists() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        Err(TestError::Mcp("Harness failed to start".into()))
    }
}
```

## Benefits

1. **Works with Xcode 16+**: Uses the new AXPTranslator APIs
2. **No entitlement issues**: Leverages Apple's signed runner
3. **Backwards compatible**: Falls back to HID when harness unavailable
4. **CI-friendly**: Can run in automated environments

## Limitations

1. **Simulator only**: Real devices need different approach
2. **Requires Xcode**: Must have Xcode installed
3. **Process overhead**: Slightly slower due to IPC

## Testing Strategy

1. **Local testing**: Run harness on developer machines
2. **CI matrix**: 
   - macOS-latest with harness (AXP path)
   - Older macOS with HID fallback
3. **Performance tests**: Measure tap latency via socket

## Next Steps

1. Create the ArkavoUITapHarness Xcode project
2. Implement Unix socket server in Swift
3. Update IDB Direct to check for harness
4. Add harness launcher to test infrastructure
5. Document usage for developers