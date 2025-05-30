# iOS Production Setup for Arkavo Edge

## Architecture Overview

In production, Arkavo Edge operates as an MCP (Model Context Protocol) server that AI agents can call to perform iOS testing and automation tasks.

```
┌─────────────────┐     MCP Protocol      ┌──────────────────┐
│                 │ ◄─────────────────────► │                  │
│   AI Agent      │                         │  Arkavo Edge     │
│   (Claude,      │     JSON-RPC over      │  MCP Server      │
│    GPT, etc)    │     stdio/HTTP         │                  │
└─────────────────┘                         └────────┬─────────┘
                                                     │
                                                     │ FFI Bridge
                                                     │
                                            ┌────────▼─────────┐
                                            │                  │
                                            │   iOS Bridge     │
                                            │   (ios_impl.c)   │
                                            │                  │
                                            └────────┬─────────┘
                                                     │
                                                     │ xcrun simctl
                                                     │
                                   ┌─────────────────┴─────────────────┐
                                   │                                   │
                           ┌───────▼────────┐                ┌────────▼───────┐
                           │                │                │                │
                           │ iOS Simulator  │                │  iOS Device    │
                           │                │                │  (via USB)     │
                           └────────────────┘                └────────────────┘
```

## Production Requirements

### Host Machine (macOS)
- **OS**: macOS 12.0 or later
- **Xcode**: 14.0 or later installed
- **Hardware**: Apple Silicon (M1/M2/M3) or Intel Mac

### iOS Target
- **Simulator**: iOS 15.0+ simulator installed via Xcode
- **Device**: iOS device connected via USB (optional)

## Running in Production

### 1. Start iOS Simulator
```bash
# List available simulators
xcrun simctl list devices

# Boot a specific simulator
xcrun simctl boot "iPhone 15 Pro"

# Or create and boot a new one
xcrun simctl create "Test iPhone" "iPhone 15 Pro" "iOS 17.2"
xcrun simctl boot "Test iPhone"
```

### 2. Start Arkavo Edge MCP Server
```bash
# From the arkavo-edge directory
cargo run --release -- serve

# Or if installed globally
arkavo serve
```

### 3. AI Agent Connection
The AI agent connects to Arkavo Edge via MCP protocol and can call tools like:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "ui_interaction",
    "arguments": {
      "action": "tap",
      "target": {
        "text": "Sign In"
      }
    }
  },
  "id": 1
}
```

## Available Tools in Production

When connected to a real iOS simulator/device, these tools provide actual functionality:

1. **ui_interaction** - Tap, swipe, type text on real UI elements
2. **screen_capture** - Take actual screenshots
3. **ui_query** - Query real accessibility trees and visible elements
4. **biometric_auth** - Trigger Face ID/Touch ID prompts
5. **system_dialog** - Handle iOS permission dialogs
6. **run_test** - Execute XCTest suites
7. **intelligent_bug_finder** - Analyze real app behavior
8. **chaos_test** - Inject real network/system failures

## Production Configuration

### Environment Variables
```bash
# Optional: Specify default simulator
export ARKAVO_IOS_DEVICE_ID="YOUR-SIMULATOR-UUID"

# Optional: Enable debug logging
export ARKAVO_LOG_LEVEL=debug
```

### MCP Server Configuration
Create `~/.arkavo/config.json`:
```json
{
  "mcp": {
    "port": 3000,
    "host": "localhost",
    "transport": "stdio"
  },
  "ios": {
    "default_simulator": "iPhone 15 Pro",
    "screenshot_dir": "./screenshots",
    "test_timeout": 30
  }
}
```

## Verifying Production Setup

Run the verification script:
```bash
# Check if everything is properly configured
arkavo verify ios

# Expected output:
# ✓ macOS version: 14.2
# ✓ Xcode installed: 15.1
# ✓ iOS Simulator available
# ✓ xcrun simctl accessible
# ✓ Active simulator: iPhone 15 Pro (Booted)
# ✓ Bridge compiled for: arm64
# ✓ MCP server ready
```

## Troubleshooting

### Simulator Not Found
```bash
# Reset simulators
xcrun simctl shutdown all
xcrun simctl erase all

# Recreate
xcrun simctl create "iPhone 15 Pro" com.apple.CoreSimulator.SimDeviceType.iPhone-15-Pro com.apple.CoreSimulator.SimRuntime.iOS-17-2
```

### Bridge Connection Issues
```bash
# Check if bridge is compiled correctly
nm target/release/arkavo | grep ios_bridge

# Verify FFI symbols
otool -L target/release/arkavo | grep CoreFoundation
```

### Permission Issues
Ensure Xcode command line tools have necessary permissions:
```bash
sudo xcode-select --switch /Applications/Xcode.app
sudo xcodebuild -license accept
```

## Production Best Practices

1. **Simulator Management**: Keep simulators in a clean state between test runs
2. **Resource Cleanup**: Implement proper cleanup in test teardown
3. **Error Handling**: AI agents should handle simulator boot failures gracefully
4. **Performance**: Use simulator snapshots for faster test initialization
5. **Security**: Run MCP server with appropriate access controls in production

## Integration Example

Here's how an AI agent might use Arkavo Edge in production:

```python
# AI Agent Code (Python example)
import mcp_client

async def test_ios_login_flow():
    # Connect to Arkavo Edge MCP server
    client = mcp_client.connect("localhost:3000")
    
    # Capture initial screen
    await client.call_tool("screen_capture", {"name": "login_screen"})
    
    # Find and tap email field
    await client.call_tool("ui_interaction", {
        "action": "tap",
        "target": {"text": "Email"}
    })
    
    # Type email
    await client.call_tool("ui_interaction", {
        "action": "type_text",
        "value": "test@example.com"
    })
    
    # Continue with test flow...
```

This setup enables AI agents to perform real iOS automation and testing tasks through the MCP protocol, with all actions executing on actual iOS simulators or devices.