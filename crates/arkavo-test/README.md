# Arkavo Test - iOS UI Automation Guide

This guide helps you use XCUITest for text-based UI automation via MCP.

## CRITICAL: XCUITest Setup Required

⚠️ **IMPORTANT**: Before using ANY text-based UI interactions (tapping buttons by text, finding elements by label), you MUST first initialize XCUITest:

```json
{
  "tool": "setup_xcuitest",
  "arguments": {
    "device_id": "YOUR-DEVICE-ID"
  }
}
```

Without this setup, text-based interactions will fail with `XCUITEST_NOT_AVAILABLE` errors.

## Prerequisites

- macOS with Xcode installed
- Node.js and npm
- At least one iOS Simulator installed
- MCP Inspector: `npx @modelcontextprotocol/inspector`

## Quick Start

### 1. Build the MCP Server

```bash
# From the arkavo-edge root directory
cd ../.. && cargo build --release --bin arkavo
```

### 2. Start MCP Inspector

```bash
npx @modelcontextprotocol/inspector
```

Open http://127.0.0.1:6274 in your browser.

### 3. Configure MCP Server

Click "+" to add a new server with this configuration:

```json
{
  "command": "cargo",
  "args": ["run", "--bin", "arkavo", "--", "mcp"],
  "env": {
    "RUST_LOG": "info"
  }
}
```

Or use the built binary:

```json
{
  "command": "./target/release/arkavo",
  "args": ["mcp"],
  "env": {
    "RUST_LOG": "info"
  }
}
```

## Proper Workflow for UI Automation

### Step 0: Initialize XCUITest FIRST

**This is mandatory!** Call `setup_xcuitest` before attempting any text-based UI interaction:

```json
{
  "tool": "setup_xcuitest", 
  "arguments": {
    "device_id": "YOUR-DEVICE-ID"
  }
}
```

Wait for a successful response before proceeding.

### Step 1: Check Available Devices

Use the `device_management` tool:

```json
{
  "action": "list"
}
```

This shows all available healthy iOS simulators. 

If you see fewer devices than expected, some may have unavailable runtimes. To diagnose:

```json
{
  "action": "health_check"
}
```

To clean up devices with missing runtimes:

```json
{
  "action": "cleanup_unhealthy",
  "dry_run": true
}
```

Remove `dry_run` or set to `false` to actually delete unhealthy devices.

### Step 2: Boot a Simulator (if needed)

If no devices are booted, use `boot_wait` to boot and wait until ready:

```json
{
  "action": "boot_wait",
  "device_id": "YOUR-DEVICE-ID-HERE",
  "timeout_seconds": 60
}
```

The `boot_wait` action will:
- Start the boot process
- Wait for device to show as "Booted"
- Check if UI services (SpringBoard) are ready
- Return detailed status including boot duration

Response includes:
- `boot_status.current_state`: "Ready", "Failed", etc.
- `boot_status.boot_duration_seconds`: Time taken to boot
- `boot_status.services_ready`: Core services status
- `boot_status.ui_ready`: UI availability

### Step 3: Check XCTest Status

Use the new `xctest_status` tool to verify XCTest functionality:

#### Check all devices:
```json
{}
```

#### Find best device for XCTest:
```json
{
  "find_best": true
}
```

#### Check specific device:
```json
{
  "device_id": "YOUR-DEVICE-ID-HERE"
}
```

Expected response includes:
- `is_functional`: Whether XCTest bridge works
- `bundle_installed`: Whether XCTest runner is installed
- `bridge_connectable`: Whether communication works
- `swift_response_time`: Connection latency

### Step 4: Setup XCTest (if needed)

If XCTest is not functional, run setup:

```json
{
  "device_id": "YOUR-DEVICE-ID-HERE"
}
```

Or force reinstall:

```json
{
  "device_id": "YOUR-DEVICE-ID-HERE",
  "force_reinstall": true
}
```

The response will include:
- Setup status
- Device XCTest status after setup
- Available capabilities

### Step 5: Verify UI Interaction

Once XCTest is set up, test UI interaction:

```json
{
  "action": "tap",
  "target": {
    "text": "Continue"
  }
}
```

## Validation Checklist

- [ ] `xctest_status` returns device information
- [ ] Can identify functional vs non-functional devices
- [ ] `setup_xcuitest` returns detailed status info
- [ ] Setup completes successfully on booted device
- [ ] After setup, `xctest_status` shows device as functional
- [ ] Response times are under 2 seconds for status checks
- [ ] Error messages are clear and actionable

## Troubleshooting

### No devices found
- Ensure Xcode is installed: `xcode-select --install`
- Check simulators: `xcrun simctl list devices`

### XCTest setup fails
- Check the error details in the response
- Ensure simulator is fully booted
- Try `force_reinstall: true`

### Bridge not connecting
- Check if test runner is installed: Look for "com.arkavo.testrunner" in device apps
- Review socket path in error messages
- Check simulator logs: `xcrun simctl spawn booted log stream`

## Performance Expectations

- Device status check: <1 second
- XCTest verification: <2 seconds
- Full setup process: <30 seconds
- UI interaction: <500ms response time

## Advanced Testing

### Run Rust Tests

For automated validation:

```bash
# Quick tests (no simulator needed)
cargo test -p arkavo-test test_xctest_quick_verify

# Full test cycle
cargo test -p arkavo-test test_fast_xctest_verification_cycle -- --nocapture

# Integration tests (requires booted simulator)
cargo test -p arkavo-test test_xctest_setup_and_verify -- --ignored
```

### Debug Logging

Set environment variable for detailed logs:

```bash
RUST_LOG=arkavo_test=debug cargo run --bin arkavo -- mcp
```

## Key Tools for Testing

1. **xctest_status** - Check XCTest functionality
2. **setup_xcuitest** - Install and configure XCTest
3. **device_management** - Manage simulators
   - `boot_wait` - Boot and wait until ready
   - `boot_status` - Check boot progress
4. **simulator_advanced** - Advanced simulator features
   - `diagnostics` - Get detailed simulator info
   - `list_processes` - See running processes
   - `clone_device` - Create device clones
5. **ui_interaction** - Test UI automation

## Expected Behavior

1. Fresh simulator: XCTest not functional
2. After setup: XCTest functional with ~100-200ms response time
3. Subsequent checks: Fast verification (<500ms)
4. Error handling: Clear messages with retry guidance