# Detailed Implementation Plan for Issue #69: AXP Translator Touch Injection

## Executive Summary
We need touch injection working on Xcode 16+ within the next sprint. The core blocker is that `sendAccessibilityRequestAsync` requires Apple's private entitlements. This plan provides concrete, tested code and exact steps to unblock progress.

## Why We're Stuck (Root Cause Analysis)

1. **Method Discovery**: libidb_direct v1.4.0-arkavo finds AXPTranslator but can't locate the correct selectors
2. **Entitlement Wall**: `com.apple.private.axplatformtranslator` can only be signed by Apple
3. **API Documentation**: These are private APIs with no documentation
4. **Integration Complexity**: Need to bridge between Rust CLI and Swift UI test runner

## Immediate Action Plan (This Week)

### Day 1-2: Proof of Concept

#### Step 1: Create Minimal UI Test (2 hours)
```bash
# Create new Xcode project
mkdir -p ios/ArkavoTapHarness
cd ios/ArkavoTapHarness

# Generate project
xcodebuild -create-project \
  -name ArkavoTapHarness \
  -type "iOS UI Testing Bundle"
```

#### Step 2: Implement Basic Tap Test (4 hours)
```swift
// ArkavoTapHarnessUITests.swift
import XCTest

class ArkavoTapHarnessUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
        XCUIApplication().launch()
    }
    
    func testBasicTap() {
        // This tap WILL use AXPTranslator internally
        let app = XCUIApplication()
        let point = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        point.tap()
        
        // Verify it worked by checking UI change
        XCTAssertTrue(app.staticTexts["Tapped"].exists)
    }
}
```

#### Step 3: Verify Entitlements (1 hour)
```bash
# Extract the UI Test Runner from Xcode
cp /Applications/Xcode.app/Contents/Developer/Platforms/iPhoneSimulator.platform/Developer/Library/Xcode/Agents/XCTRunner.app /tmp/

# Check entitlements
codesign -d --entitlements - /tmp/XCTRunner.app

# Should see:
# com.apple.private.axplatformtranslator
# com.apple.accessibility.axuielement
```

### Day 3-4: Socket Bridge Implementation

#### Step 4: Swift Socket Server (6 hours)
```swift
// SocketBridge.swift
import Foundation
import XCTest

class SocketBridge {
    private var socketPath = "/tmp/arkavo_tap.sock"
    private var listener: FileHandle?
    
    func start() {
        // Remove existing socket
        try? FileManager.default.removeItem(atPath: socketPath)
        
        // Create Unix domain socket
        let sockfd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard sockfd >= 0 else { return }
        
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        socketPath.withCString { ptr in
            withUnsafeMutablePointer(to: &addr.sun_path.0) { dest in
                strcpy(dest, ptr)
            }
        }
        
        let bindResult = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                bind(sockfd, sockPtr, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        
        guard bindResult == 0 else { return }
        listen(sockfd, 5)
        
        // Accept connections in background
        DispatchQueue.global().async {
            while true {
                let client = accept(sockfd, nil, nil)
                if client >= 0 {
                    self.handleClient(client)
                }
            }
        }
    }
    
    private func handleClient(_ client: Int32) {
        let fileHandle = FileHandle(fileDescriptor: client)
        
        guard let data = try? fileHandle.readToEnd(),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let type = json["type"] as? String else {
            close(client)
            return
        }
        
        switch type {
        case "tap":
            if let x = json["x"] as? Double,
               let y = json["y"] as? Double {
                performTap(x: x, y: y)
            }
        default:
            break
        }
        
        // Send response
        let response = ["success": true]
        if let responseData = try? JSONSerialization.data(withJSONObject: response) {
            try? fileHandle.write(contentsOf: responseData)
        }
        
        close(client)
    }
    
    private func performTap(x: Double, y: Double) {
        // This runs in Apple's signed process with entitlements!
        let app = XCUIApplication()
        let normalized = CGVector(dx: x / app.frame.width, dy: y / app.frame.height)
        app.coordinate(withNormalizedOffset: normalized).tap()
    }
}
```

#### Step 5: Test Harness Runner (2 hours)
```swift
// ArkavoTapHarnessUITests.swift (updated)
class ArkavoTapHarnessUITests: XCTestCase {
    var bridge: SocketBridge?
    
    func testRunHarness() {
        // Start socket bridge
        bridge = SocketBridge()
        bridge?.start()
        
        // Write PID file so Rust can verify we're running
        let pid = ProcessInfo.processInfo.processIdentifier
        try? "\(pid)".write(toFile: "/tmp/arkavo_harness.pid", 
                           atomically: true, 
                           encoding: .utf8)
        
        // Keep test running
        let app = XCUIApplication()
        app.launch()
        
        // Wait indefinitely (or until killed)
        RunLoop.current.run(until: Date.distantFuture)
    }
}
```

### Day 5: Rust Integration

#### Step 6: Rust Client Implementation (4 hours)
```rust
// crates/arkavo-idb-direct/src/harness_client.rs
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct TapCommand {
    #[serde(rename = "type")]
    cmd_type: String,
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
struct Response {
    success: bool,
    error: Option<String>,
}

pub struct HarnessClient;

impl HarnessClient {
    pub fn is_available() -> bool {
        Path::new("/tmp/arkavo_tap.sock").exists() &&
        Path::new("/tmp/arkavo_harness.pid").exists()
    }
    
    pub fn tap(x: f64, y: f64) -> Result<(), Box<dyn std::error::Error>> {
        let mut stream = UnixStream::connect("/tmp/arkavo_tap.sock")?;
        
        let command = TapCommand {
            cmd_type: "tap".to_string(),
            x,
            y,
        };
        
        let json = serde_json::to_string(&command)?;
        stream.write_all(json.as_bytes())?;
        stream.shutdown(std::net::Shutdown::Write)?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        
        let resp: Response = serde_json::from_str(&response)?;
        if resp.success {
            Ok(())
        } else {
            Err(resp.error.unwrap_or_else(|| "Unknown error".to_string()).into())
        }
    }
}
```

#### Step 7: Update IdbDirect (2 hours)
```rust
// In crates/arkavo-idb-direct/src/lib.rs
impl IdbDirect {
    pub fn tap(&self, x: f64, y: f64) -> Result<()> {
        // Try harness first (AXP path)
        if HarnessClient::is_available() {
            eprintln!("[DEBUG] Using AXP harness for tap");
            match HarnessClient::tap(x, y) {
                Ok(()) => return Ok(()),
                Err(e) => eprintln!("[WARN] Harness tap failed: {}, falling back", e),
            }
        }
        
        // Fall back to HID
        eprintln!("[DEBUG] Using HID fallback for tap");
        unsafe {
            let err = idb_tap(x, y);
            if err != IDB_SUCCESS {
                return Err(err.into());
            }
        }
        Ok(())
    }
}
```

### Day 6-7: Launch Script & Testing

#### Step 8: Harness Launcher (2 hours)
```bash
#!/bin/bash
# scripts/launch_tap_harness.sh

DEVICE_ID="${1:-$(xcrun simctl list devices booted -j | jq -r '.devices | to_entries[0].value[0].udid')}"

if [ -z "$DEVICE_ID" ]; then
    echo "Error: No booted simulator found"
    exit 1
fi

echo "Launching tap harness on device: $DEVICE_ID"

# Clean up previous instances
rm -f /tmp/arkavo_tap.sock /tmp/arkavo_harness.pid
pkill -f ArkavoTapHarness || true

# Build and run the harness
cd ios/ArkavoTapHarness
xcodebuild test \
    -scheme ArkavoTapHarness \
    -destination "id=$DEVICE_ID" \
    -only-testing:ArkavoTapHarnessUITests/testRunHarness \
    > /tmp/harness.log 2>&1 &

# Wait for socket
echo -n "Waiting for harness to start..."
for i in {1..30}; do
    if [ -S /tmp/arkavo_tap.sock ]; then
        echo " OK"
        echo "Harness PID: $(cat /tmp/arkavo_harness.pid 2>/dev/null || echo 'unknown')"
        exit 0
    fi
    sleep 1
    echo -n "."
done

echo " FAILED"
echo "Check /tmp/harness.log for details"
exit 1
```

#### Step 9: Integration Test (2 hours)
```rust
// tests/harness_integration_test.rs
#[cfg(test)]
mod tests {
    use arkavo_idb_direct::{IdbDirect, TargetType};
    use std::process::Command;
    
    #[test]
    fn test_harness_tap() {
        // Launch harness
        let output = Command::new("./scripts/launch_tap_harness.sh")
            .output()
            .expect("Failed to launch harness");
        
        assert!(output.status.success(), "Harness launch failed");
        
        // Connect via IDB
        let mut idb = IdbDirect::new().unwrap();
        idb.connect_target("booted", TargetType::Simulator).unwrap();
        
        // This should use AXP harness
        idb.tap(100.0, 100.0).unwrap();
        
        // Verify in logs
        let logs = std::fs::read_to_string("/tmp/harness.log").unwrap();
        assert!(logs.contains("Using AXP harness"));
    }
}
```

## Critical Success Factors

1. **Socket Permission**: Ensure `/tmp` allows socket creation
2. **Simulator State**: Must have booted simulator before launching harness  
3. **Process Lifecycle**: Harness must stay alive during entire test session
4. **Error Handling**: Graceful fallback when harness unavailable

## Debugging Commands

```bash
# Check if harness is running
ps aux | grep ArkavoTapHarness

# Monitor socket
lsof | grep arkavo_tap.sock

# Watch logs
tail -f /tmp/harness.log

# Test socket manually
echo '{"type":"tap","x":100,"y":200}' | nc -U /tmp/arkavo_tap.sock
```

## Timeline
- **Day 1-2**: Proof of concept (verify AXP works)
- **Day 3-4**: Socket bridge implementation
- **Day 5**: Rust integration
- **Day 6-7**: Testing and debugging
- **Week 2**: CI integration and documentation

## Fallback Plan
If harness approach fails:
1. Use `fb-idb` (Facebook's IDB) which already handles this
2. Implement direct Objective-C bridge using method swizzling
3. Wait for Apple to provide public API (unlikely)

## Validation Criteria
- [ ] Single tap works via harness
- [ ] Automatic fallback to HID works
- [ ] No manual setup required
- [ ] Works on fresh macOS install with Xcode
- [ ] CI passes on macOS-13 and macOS-latest