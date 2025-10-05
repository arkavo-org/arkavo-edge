# Immediate Steps to Unblock Touch Injection (Today)

## The Key Insight You Might Be Missing

The AXPTranslator methods are NOT exposed to your binary. They're only available INSIDE Apple's XCTRunner.app. You must:

1. **Stop trying to call AXPTranslator directly from libidb_direct**
2. **Start using XCTest's coordinate.tap() which calls AXPTranslator internally**

## Minimal Working Example (1 Hour)

### 1. Create this file: `TestTap.swift`
```swift
import XCTest

class TestTap: XCTestCase {
    func testTap() {
        let app = XCUIApplication()
        app.launch()
        
        // This AUTOMATICALLY uses AXPTranslator with Apple's entitlements
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        
        sleep(10) // Keep alive to see it work
    }
}
```

### 2. Run it:
```bash
# Make sure simulator is booted first
xcrun simctl boot "iPhone 16 Pro Max" || true

# This runs INSIDE Apple's signed runner with entitlements
xcodebuild test \
  -scheme YourApp \
  -destination 'platform=iOS Simulator,name=iPhone 16 Pro Max' \
  -only-testing:YourAppUITests/TestTap/testTap
```

### 3. Verify it worked:
The tap will succeed because `xcodebuild test` uses `/Applications/Xcode.app/.../XCTRunner.app` which has the required entitlements.

## Why Your Current Approach Fails

```
Your Binary → libidb_direct → AXPTranslator ❌ (No entitlements)
                     ↓
              "No compatible touch API found"

vs.

xcodebuild → XCTRunner.app → XCTest → AXPTranslator ✅ (Has entitlements)
   (Apple)    (Apple-signed)           (Works!)
```

## The Socket Bridge Solves This

Instead of calling AXPTranslator directly, you:
1. Launch XCTest runner (which has entitlements)
2. Send tap commands via socket
3. XCTest performs the tap using its privileged access

## Next Action (Right Now)

1. Confirm the minimal example above works
2. If yes → implement socket bridge
3. If no → check Xcode installation

This is why fb-idb works - it uses a similar approach with a companion process.