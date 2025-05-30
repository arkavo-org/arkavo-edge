# XCUITest Integration for arkavo-test

This document describes the XCUITest integration that provides reliable iOS UI automation for the arkavo-test framework.

## Overview

The XCUITest integration addresses GitHub Issue #10 by implementing a native iOS testing solution that can:
- Tap UI elements by text content
- Tap UI elements by accessibility ID
- Tap by coordinates with accurate mapping
- Return detailed element information

## Architecture

```
┌─────────────────────┐     Unix Socket      ┌─────────────────────┐
│                     │ ◄─────────────────►   │                     │
│   Rust MCP Server   │                       │  Swift XCUITest     │
│  (arkavo-test)      │    JSON Commands      │     Runner          │
│                     │ ─────────────────►    │                     │
│                     │    JSON Responses     │                     │
│                     │ ◄─────────────────    │                     │
└─────────────────────┘                       └─────────────────────┘
```

### Components

1. **XCTest Compiler** (`xctest_compiler.rs`)
   - Dynamically compiles Swift XCUITest code
   - No Xcode project required
   - Caches compiled bundles

2. **Unix Socket Bridge** (`xctest_unix_bridge.rs`)
   - High-performance local IPC
   - Bidirectional communication
   - Automatic cleanup

3. **Swift Test Runner** (`ArkavoTestRunner.swift.template`)
   - Native XCUITest implementation
   - Handles tap commands
   - Returns element information

## Usage

### Basic Example

```rust
use arkavo_test::mcp::xctest_unix_bridge::XCTestUnixBridge;

// Create and start bridge
let mut bridge = XCTestUnixBridge::new();
bridge.start().await?;

// Tap by text
let tap_cmd = XCTestUnixBridge::create_text_tap(
    "Login".to_string(),
    Some(5.0) // timeout
);
let response = bridge.send_tap_command(tap_cmd).await?;

// Tap by accessibility ID
let tap_cmd = XCTestUnixBridge::create_accessibility_tap(
    "login_button".to_string(),
    None
);
let response = bridge.send_tap_command(tap_cmd).await?;
```

### Integration with MCP

The `UiInteractionKit` in `ios_tools.rs` is set up to use XCUITest when available:

```rust
// When a tap with text is requested:
{
    "action": "tap",
    "target": {
        "text": "Login"
    }
}
```

The system will:
1. Check if XCUITest runner is available
2. If not, compile and install it
3. Execute tap using native XCUITest
4. Fall back to AppleScript if XCUITest fails

## Benefits

### Over AppleScript/Coordinate-based approach:
- ✅ **Text-based tapping**: Find and tap elements by their text content
- ✅ **Accessibility ID support**: Use developer-defined identifiers
- ✅ **Element information**: Get type, frame, and other properties
- ✅ **Better reliability**: Native framework vs. screen coordinate mapping
- ✅ **Proper error messages**: Know exactly why a tap failed

### Performance (Unix Sockets):
- Lower latency than HTTP
- No network stack overhead
- Secure local communication
- Standard macOS IPC pattern

## Requirements

- macOS with Xcode Command Line Tools
- iOS Simulator
- Swift 5.0+

## Testing

Run sanity checks:
```bash
cargo test --package arkavo-test --test xctest_basic_sanity
```

Run integration tests:
```bash
cargo test --package arkavo-test --test xctest_integration
```

## Implementation Status

- ✅ Swift template with XCUITest runner
- ✅ Unix socket communication
- ✅ Dynamic compilation
- ✅ Tap by text
- ✅ Tap by accessibility ID
- ✅ Tap by coordinates
- ✅ Error handling
- ✅ Fallback to AppleScript

## Future Enhancements

- [ ] Additional UI actions (swipe, type text, etc.)
- [ ] Element query API
- [ ] Screenshot with element highlighting
- [ ] Performance optimizations (reuse runner)
- [ ] Device rotation support

## Troubleshooting

### "Failed to compile XCTest bundle"
- Ensure Xcode Command Line Tools are installed: `xcode-select --install`
- Check Swift version: `swift --version`

### "Cannot connect to Unix socket"
- Check permissions on socket file
- Ensure no stale socket files in `/tmp`
- Verify XCTest runner started successfully

### "Element not found"
- Use `analyze_layout` action to capture screenshot
- Verify element has text or accessibility ID
- Check if element is visible on screen

## References

- [XCUITest Documentation](https://developer.apple.com/documentation/xctest/xcuiapplication)
- [Unix Domain Sockets](https://man7.org/linux/man-pages/man7/unix.7.html)
- Similar approaches: WebDriverAgent, Maestro, Appium