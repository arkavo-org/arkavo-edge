# iOS Simulator UI Automation Improvements

## Current Problems

1. **Hard-coded Bezel Estimates**: The current AppleScript implementation uses hard-coded bezel sizes (70px sides, 85px top, 120px bottom) that are fragile and break with different simulator window scales or device types.

2. **Invalid simctl Commands**: The codebase previously attempted to use non-existent `xcrun simctl io tap` commands that don't exist.

3. **AppleScript Limitations**:
   - Requires guessing the device screen position within the simulator window
   - Bezel sizes vary by device type and simulator scale
   - No reliable way to get exact device screen bounds
   - Fails when simulator window is scaled or positioned differently

## Available Solutions

### 1. XCTest Bridge (Already Implemented - RECOMMENDED)

The codebase already has a robust XCTest bridge implementation that:
- Communicates via Unix socket with a test runner on the simulator
- Supports coordinate-based tapping without bezel calculations
- Can find elements by text or accessibility ID
- Handles swipes, typing, and other gestures
- Works reliably regardless of simulator window position/scale

**Usage**:
```rust
// Initialize XCTest (already done in setup_xcuitest tool)
let bridge = XCTestUnixBridge::new();
bridge.start().await?;

// Tap by coordinates (device coordinates, not window)
bridge.tap(200.0, 400.0).await?;

// Tap by text (finds element automatically)
bridge.tap_by_text("Login", Some(10.0)).await?;
```

### 2. Programmatic Device Screen Bounds

Unfortunately, there's no direct API to get the exact device screen position within the simulator window. The attempted approaches:
- `xcrun simctl` doesn't provide window/screen position info
- AppleScript Accessibility API can find the window but not reliably identify the device screen area
- The device screen is an AXGroup element but not consistently identifiable

### 3. Alternative Approaches

#### A. Focus on XCTest (Recommended)
The XCTest bridge is already implemented and working. It should be the primary method for UI automation because:
- It works directly on device coordinates (not window coordinates)
- No bezel calculations needed
- Supports text-based element finding
- More reliable than AppleScript

#### B. Use `xcrun simctl io screenshot` for Visual Analysis
For cases where you need to understand the UI:
1. Capture screenshot: `xcrun simctl io <device> screenshot`
2. Use AI vision to analyze the screenshot
3. Use XCTest to tap at the identified coordinates

#### C. Avoid Coordinate-Based Tapping When Possible
- Always prefer text-based or accessibility ID-based tapping
- Use `analyze_layout` to capture screenshots and identify elements
- Only fall back to coordinates when elements can't be found by text

## Implementation Recommendations

1. **Remove AppleScript Tap Implementation**: The current AppleScript-based tapping in `ios_tools.rs` should be deprecated in favor of XCTest.

2. **Make XCTest Setup More Prominent**: The `setup_xcuitest` tool should be called automatically or made more obvious to users.

3. **Improve Error Messages**: When XCTest isn't available, provide clear instructions on running `setup_xcuitest`.

4. **Document Device Coordinate Systems**: Make it clear that XCTest uses device coordinates (logical points), not window coordinates.

## Example Workflow

```bash
# 1. Setup XCTest (one time)
arkavo mcp setup_xcuitest

# 2. Analyze screen to find elements
arkavo mcp ui_interaction --action analyze_layout

# 3. Use text-based tapping (preferred)
arkavo mcp ui_interaction --action tap --target '{"text": "Sign In"}'

# 4. Or use coordinates from screenshot analysis
arkavo mcp ui_interaction --action tap --target '{"x": 200, "y": 400}'
```

## Technical Details

### XCTest Bridge Implementation
- Location: `crates/arkavo-test/src/mcp/xctest_unix_bridge.rs`
- Uses Unix socket for communication
- Compiles Swift test runner on demand
- Supports all major UI interactions

### Coordinate Systems
- **Device Coordinates**: Logical points (e.g., iPhone 15 Pro is 393x852 points)
- **Pixel Coordinates**: Physical pixels (multiply by scale factor, usually 2x or 3x)
- **Window Coordinates**: macOS window position (includes bezels and chrome)

### Device Resolutions (Logical Points)
- iPhone 16 Pro Max: 430x932
- iPhone 16 Pro: 393x852
- iPhone 16: 390x844
- iPhone SE: 375x667
- iPad: 820x1180

## Conclusion

The XCTest bridge is the most reliable solution for iOS simulator automation. It avoids the fragility of coordinate transformations and bezel calculations by working directly with the simulator's test framework. The AppleScript approach should be phased out in favor of XCTest.