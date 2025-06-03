# Enhanced iOS Testing with XCTest Bridge

This document describes the enhanced iOS testing capabilities that support tap, swipe, and type_text commands without requiring accessibility permissions.

## Overview

The enhanced iOS testing system uses XCTest to interact with iOS simulators, providing reliable UI automation without macOS accessibility permission requirements.

## Architecture

1. **XCTest Bridge**: A Unix socket-based communication bridge between Rust and Swift
2. **XCTest Runner**: A Swift test bundle that runs on the simulator and executes commands
3. **Enhanced Command Support**: Full support for tap, swipe, type_text, scroll, and long press

## Supported Commands

### Tap Commands

```rust
// Tap at coordinates
xctest.tap(200.0, 400.0).await?;

// Tap element by text
xctest.tap_by_text("Login", Some(5.0)).await?;

// Tap element by accessibility ID
xctest.tap_by_accessibility_id("login_button", Some(5.0)).await?;
```

### Swipe Commands

```rust
// Create swipe command
let swipe_cmd = XCTestUnixBridge::create_swipe(
    200.0, 600.0,  // Start (x1, y1)
    200.0, 200.0,  // End (x2, y2)
    Some(0.5)      // Duration in seconds
);

// Send swipe command
bridge.send_command(swipe_cmd).await?;
```

### Type Text Commands

```rust
// Type text (with option to clear existing text first)
let type_cmd = XCTestUnixBridge::create_type_text(
    "Hello, World!".to_string(),
    true  // Clear existing text first
);

bridge.send_command(type_cmd).await?;
```

## Setup Instructions

### Prerequisites

- Xcode installed
- iOS Simulator available
- Swift 5.0 or later

### Installation

1. The XCTest runner will be automatically compiled when first used
2. Templates are located in `crates/arkavo-test/templates/XCTestRunner/`
3. The compiled bundle is cached for subsequent runs

## Usage in MCP Tools

The iOS tools in the MCP server will automatically use XCTest when available:

```json
{
  "action": "tap",
  "target": {"x": 200, "y": 400}
}

{
  "action": "swipe",
  "swipe": {
    "x1": 200, "y1": 600,
    "x2": 200, "y2": 200,
    "duration": 0.5
  }
}

{
  "action": "type_text",
  "value": "Hello, World!"
}
```

## Troubleshooting

### XCTest Runner Not Starting

1. Check that Xcode is installed: `xcode-select -p`
2. Ensure simulator is booted: `xcrun simctl list devices | grep Booted`
3. Check logs for compilation errors

### Commands Not Working

1. Verify the app is launched on the simulator
2. Check that coordinates are within screen bounds
3. Ensure XCTest bundle is properly installed

### Socket Connection Issues

1. Check socket path permissions
2. Ensure no firewall is blocking Unix sockets
3. Verify the socket file exists at `/tmp/arkavo-xctest-*.sock`

## Implementation Details

### Command Protocol

Commands are sent as JSON over Unix socket:

```json
{
  "id": "unique-id",
  "type": "tap|swipe|typeText|scroll|longPress",
  "parameters": {
    // Command-specific parameters
  }
}
```

### Response Format

```json
{
  "id": "command-id",
  "success": true,
  "error": null,
  "result": {
    // Command-specific results
  }
}
```

## Benefits Over AppleScript

1. **No Accessibility Permissions Required**: Works without system-level permissions
2. **More Reliable**: Direct integration with XCTest framework
3. **Better Performance**: Native execution on simulator
4. **Rich Features**: Support for complex gestures and interactions
5. **Element Detection**: Can find elements by text or accessibility ID

## Future Enhancements

- Support for multi-touch gestures
- Screenshot capture with element highlighting
- Accessibility tree inspection
- Performance metrics collection