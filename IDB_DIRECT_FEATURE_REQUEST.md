# Feature Request: Touch and Screenshot API Support for libidb_direct v1.3.2-arkavo.1

## Summary
While v1.3.2-arkavo.1 successfully fixes the segmentation fault, the touch (tap/swipe) and screenshot APIs are not functioning on macOS with Xcode 16.2.

## Current Status
- ✅ Connection to simulator works without crashing
- ❌ Touch APIs (tap, swipe) fail with "No compatible touch API found"
- ❌ Screenshot API fails with "Screenshot failed: (null)"

## Environment
- **OS**: macOS 15.5 (24F74)
- **Architecture**: Apple Silicon (arm64)
- **Xcode Version**: 16.2 (Build 16C5032a)
- **libidb_direct Version**: v1.3.2-arkavo.1
- **CoreSimulator Version**: 1031.0.0
- **Simulator**: iPhone 16 Pro Max, iOS 18.2 (Booted)
- **Device UDID**: F76602B2-EC91-4A32-BBAA-E36ADDBF83C4
- **Device Type**: com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro-Max
- **Runtime**: com.apple.CoreSimulator.SimRuntime.iOS-18-2

## Detailed Error Analysis

### 1. Touch API Error
When calling `idb_tap(195.0, 422.0)`:
```
2025-06-13 20:05:40.802 test_tap_only[62277:81757050] No compatible touch API found
Error: OperationFailed
```

This suggests the library is unable to find or use the appropriate CoreSimulator touch event APIs.

### 2. Screenshot API Error
When calling `idb_take_screenshot()`:
```
2025-06-13 20:05:10.606 tap_test[62226:81756097] Screenshot failed: (null)
Error: OperationFailed
```

The "(null)" error message indicates the native code is not properly returning error details.

## Potential Root Causes

### 1. Private API Changes
Xcode 16.2 may have changed private CoreSimulator APIs for:
- Touch event injection (`SimDevice` touch methods)
- Screenshot capture mechanisms

### 2. Missing Framework Initialization
The library might need to:
- Initialize additional CoreSimulator subsystems
- Load specific private frameworks dynamically
- Set up proper device contexts before touch/screenshot operations

### 3. API Compatibility Detection
The "No compatible touch API found" suggests the library has compatibility detection code that's failing to identify the correct API version.

## Diagnostic Information

### CoreSimulator Framework Location
```bash
/Library/Developer/PrivateFrameworks/CoreSimulator.framework
```

### Simulator State
```
Device is booted and responsive
State number: 3 (Booted state)
Connection successful via idb_connect_target
```

### Debug Output from Library
```
2025-06-13 20:05:40.799 test_tap_only[62277:81757050] [DEBUG] stateNumber = 3 (class: __NSCFNumber)
2025-06-13 20:05:40.799 test_tap_only[62277:81757050] [DEBUG] Device state = 3
2025-06-13 20:05:40.799 test_tap_only[62277:81757050] Connected to simulator: F76602B2-EC91-4A32-BBAA-E36ADDBF83C4
```

## Recommendations for Investigation

### 1. Touch API Support
- Check if `SimDevice` class has changed touch event methods in Xcode 16.2
- Verify the method signatures for:
  - `sendTouchEvent:` or similar touch injection methods
  - Event coordinate system (logical vs physical pixels)
- Consider implementing multiple API detection strategies

### 2. Screenshot API Support
- Investigate current screenshot capture methods in CoreSimulator
- Check if the API now requires additional permissions or setup
- Verify image format and buffer handling

### 3. Enhanced Error Reporting
- Return specific error messages instead of "(null)"
- Add debug logging for API detection/selection process
- Include Xcode version detection to choose appropriate APIs

## Test Code to Reproduce

```rust
use arkavo_idb_direct::{IdbDirect, TargetType};

fn main() {
    let mut idb = IdbDirect::new().expect("Failed to initialize");
    println!("Version: {}", IdbDirect::version());

    idb.connect_target("F76602B2-EC91-4A32-BBAA-E36ADDBF83C4", TargetType::Simulator)
        .expect("Failed to connect");
    
    // This fails with "No compatible touch API found"
    match idb.tap(195.0, 422.0) {
        Ok(()) => println!("Tap successful"),
        Err(e) => eprintln!("Tap failed: {:?}", e),
    }
    
    // This fails with "Screenshot failed: (null)"
    match idb.take_screenshot() {
        Ok(screenshot) => println!("Screenshot: {}x{}", screenshot.width, screenshot.height),
        Err(e) => eprintln!("Screenshot failed: {:?}", e),
    }
}
```

## Alternative Approaches to Consider

1. **Dynamic API Resolution**: Implement runtime detection of available CoreSimulator methods
2. **Fallback Mechanisms**: Try multiple touch/screenshot APIs in sequence
3. **Version-Specific Builds**: Create Xcode version-specific variants of the library
4. **Public API Usage**: Consider using XCTest or Accessibility APIs as alternatives

## Additional Context

The IDB companion backend works correctly for these operations, suggesting the underlying simulator supports these features. The issue is specific to the direct FFI implementation's interaction with CoreSimulator private APIs.

## Priority
High - These are core features required for UI automation testing