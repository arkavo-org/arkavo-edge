# Invalid simctl Commands Found in Codebase

## Discovery
The codebase was using several invalid `simctl io` commands that do not exist:

### Invalid Commands Used:
1. `xcrun simctl io <device> tap <x> <y>` - **DOES NOT EXIST**
2. `xcrun simctl io <device> touch <x>,<y>` - **DOES NOT EXIST**
3. `xcrun simctl io <device> swipe <x1>,<y1> <x2>,<y2>` - **DOES NOT EXIST**
4. `xcrun simctl io <device> sendkey <keycode>` - **DOES NOT EXIST**

### Valid simctl io Commands:
The only valid `simctl io` operations are:
- `enumerate` - Lists IO ports
- `poll` - Polls IO ports for events
- `recordVideo` - Records screen to video file
- `screenshot` - Captures screenshot

## Files Affected:
1. `/crates/arkavo-test/src/mcp/xctest_enhanced.rs` - SimctlInteraction struct (FIXED)
2. `/crates/arkavo-test/src/mcp/passkey_dialog_handler.rs` - tap and sendkey commands (PARTIALLY FIXED)
3. `/crates/arkavo-test/src/mcp/biometric_test_scenarios.rs` - sendkey commands (NOT FIXED)
4. `/crates/arkavo-test/src/mcp/biometric_dialog_handler.rs` - sendkey commands (NOT FIXED)
5. `/crates/arkavo-test/src/mcp/ios_biometric_tools.rs` - sendkey commands (NOT FIXED)

## Required Alternatives:
For UI interactions on iOS Simulator, use:
1. **XCTest Bridge** - Via XCTestUnixBridge for tap/swipe commands
2. **AppleScript** - Using System Events to send key codes
3. **idb (iOS Development Bridge)** - Third-party tool with UI interaction support

## Key Code Reference for AppleScript:
- ESC: key code 53
- Return/Enter: key code 36
- Tab: key code 48
- Space: key code 49
- Delete: key code 51

## Example Fix:
```rust
// INVALID:
Command::new("xcrun")
    .args(["simctl", "io", &device_id, "sendkey", "escape"])
    .output()

// VALID Alternative using AppleScript:
let script = r#"
    tell application "Simulator"
        activate
        tell application "System Events"
            key code 53 -- ESC key
        end tell
    end tell
"#;
Command::new("osascript")
    .args(["-e", script])
    .output()
```

## Status:
- ✅ Fixed `xctest_enhanced.rs` - Removed invalid SimctlInteraction struct, added module documentation
- ✅ Fixed `passkey_dialog_handler.rs` - All invalid tap and sendkey commands replaced with AppleScript alternatives
- ❌ Still need to fix sendkey commands in:
  - `biometric_test_scenarios.rs`
  - `biometric_dialog_handler.rs`
  - `ios_biometric_tools.rs`

## Changes Made:
1. Removed `SimctlInteraction` struct that used invalid commands
2. Replaced `simctl io tap` with error messages suggesting ui_interaction tool
3. Replaced `simctl io sendkey` with AppleScript key code commands
4. Added proper documentation about valid simctl commands