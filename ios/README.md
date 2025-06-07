# iOS Testing with Arkavo MCP Server

## Installation

```bash
# Install the Arkavo CLI
cargo install --path .

# Start the MCP server for iOS testing
arkavo mcp --ios
```

## Using the MCP Server

The MCP server provides tools for iOS simulator automation:

### Available Tools
- `device_management` - List and manage iOS simulators
- `ui_interaction` - Tap, swipe, and interact with UI elements
- `screen_capture` - Take screenshots with optional analysis
- `biometric_auth` - Handle Face ID/Touch ID authentication
- `enrollment_dialog` - Handle biometric enrollment dialogs
- `ui_element_handler` - Advanced interaction for challenging UI elements

### Example Usage

```bash
# List available devices
arkavo mcp call device_management --action list

# Take a screenshot
arkavo mcp call screen_capture --name test_screen

# Tap a button
arkavo mcp call ui_interaction --action tap --target '{"x": 100, "y": 200}'

# Handle biometric dialog
arkavo mcp call biometric_auth --action cancel
```

## Reference App

The `ArkavoReference` directory contains an iOS app designed for testing the MCP server's capabilities. Build and run it in Xcode to validate automation tools.

## Requirements

- macOS with Xcode installed
- iOS Simulator
- Rust toolchain for building Arkavo

## Troubleshooting

- **"No devices found"** - Ensure iOS Simulator is running
- **"Tap failed"** - Check coordinates are within device bounds
- **"Biometric not available"** - Enable Face ID in Simulator settings