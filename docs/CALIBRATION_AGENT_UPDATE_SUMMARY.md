# Calibration Agent Update Summary

## Problem
The agent was trying to use `setup_xcuitest` which would fail with connection errors because it was designed for a different test host app, not the ArkavoReference app used for calibration.

## Solution
Updated the MCP server to properly use the ArkavoReference app for calibration:

### 1. Fixed Bundle IDs
- Changed from `com.arkavo.testhost` to `com.arkavo.ArkavoReference`
- Removed deprecation warnings about `com.arkavo.reference`

### 2. Added App Installation
- Added `install_reference_app` action to `calibration_manager`
- Builds and installs ArkavoReference app using xcodebuild
- No need for `setup_xcuitest` anymore

### 3. Updated Navigation
- Uses deep links (`arkavo-edge://calibration`) instead of environment variables
- Properly launches ArkavoReference app before opening deep links

### 4. Fixed Error Messages
- Updated all error messages to reference ArkavoReference app
- Removed references to `setup_xcuitest` in calibration context

## New Workflow for Agents

```json
// Step 1: Install ArkavoReference app
{
  "tool": "calibration_manager",
  "parameters": {
    "action": "install_reference_app",
    "device_id": "YOUR_DEVICE_ID"
  }
}

// Step 2: Start log streaming
{
  "tool": "log_stream",
  "parameters": {
    "action": "start",
    "process_name": "ArkavoReference"
  }
}

// Step 3: Start calibration
{
  "tool": "calibration_manager",
  "parameters": {
    "action": "start_calibration",
    "device_id": "YOUR_DEVICE_ID"
  }
}
```

## Files Modified
1. `calibration_setup_tool.rs` - Uses ArkavoReference app
2. `calibration/reference_app.rs` - Correct bundle ID and deep links
3. `calibration/server.rs` - Removed deprecation warnings
4. `calibration_tools.rs` - Added install_reference_app action

## Key Points
- **DO NOT use setup_xcuitest** for calibration
- ArkavoReference app provides visual feedback during calibration
- Full feedback loop with log streaming and diagnostic export
- Calibration should now progress properly instead of being stuck in "initializing"