# MCP Server Calibration Feedback Loop with ArkavoReference App

This document demonstrates the complete feedback loop between the MCP server and the ArkavoReference app during calibration and testing.

## Overview

The feedback loop enables the MCP server to:
1. Launch ArkavoReference in diagnostic/calibration mode
2. Stream logs from the app in real-time
3. Export diagnostic data via deep links
4. Use idb_companion for UI interactions
5. Verify calibration accuracy through the app's diagnostic overlay

## Key Components

### ArkavoReference App Features
- **Diagnostic Mode**: Enabled by default in debug builds
- **Deep Link Support**: 
  - `arkavo-edge://calibration` - Opens calibration screen
  - `arkavo-reference://diagnostic/export` - Exports diagnostic data
  - `arkavo-reference://diagnostic/enable` - Enables diagnostic overlay
- **Diagnostic Overlay**: Shows grid, tap indicators, coordinates, and event log
- **Event Logging**: Logs all interactions with timestamps and coordinates

### MCP Server Tools
- **log_stream**: Captures app console output in real-time
- **app_diagnostic_export**: Triggers diagnostic data export
- **deep_link**: Opens URLs to navigate the app
- **app_launcher**: Launches apps with environment variables
- **calibration_manager**: Manages calibration sequences

## Complete Feedback Loop Example

### Step 1: Start Log Stream
```json
{
  "tool": "log_stream",
  "parameters": {
    "action": "start",
    "process_name": "ArkavoReference"
  }
}
```

### Step 2: Launch App in Calibration Mode
```json
{
  "tool": "deep_link",
  "parameters": {
    "url": "arkavo-edge://calibration",
    "bundle_id": "com.arkavo.ArkavoReference"
  }
}
```

### Step 3: Run Calibration
```json
{
  "tool": "calibration_manager",
  "parameters": {
    "action": "run",
    "device_id": "<device_id>",
    "script_name": "reference_app_calibration"
  }
}
```

### Step 4: Export Diagnostic Data
```json
{
  "tool": "app_diagnostic_export",
  "parameters": {
    "bundle_id": "com.arkavo.ArkavoReference"
  }
}
```

### Step 5: Read Log Stream
```json
{
  "tool": "log_stream",
  "parameters": {
    "action": "read",
    "limit": 50
  }
}
```

## Verification Flow

1. **Pre-Calibration Check**
   - Launch app with diagnostic overlay enabled
   - Verify grid overlay is visible
   - Check that tap indicators appear on interactions

2. **Calibration Execution**
   - Run calibration sequence
   - Monitor logs for tap events with coordinates
   - Verify each calibration point is registered

3. **Post-Calibration Validation**
   - Export diagnostic data
   - Analyze tap accuracy from logged coordinates
   - Compare expected vs actual tap locations

4. **Test Target App**
   - Switch to target app testing
   - Use calibrated coordinates for interactions
   - Monitor success rates through logs

## Log Format

The app logs diagnostic events in JSON format:
```json
{
  "timestamp": "2024-01-20T10:30:45Z",
  "eventType": "tap",
  "eventMessage": "Tap at (195, 422)",
  "location": {"x": 195, "y": 422},
  "identifier": "checkbox_1",
  "details": "Checkbox toggled: true"
}
```

## Integration with idb_companion

The MCP server uses idb_companion for:
- Taking screenshots during calibration
- Performing taps at calibrated coordinates
- Verifying UI element visibility
- Recording test sessions

## Best Practices

1. **Always Start Log Stream First**
   - Ensures no diagnostic data is missed
   - Captures app launch events

2. **Use Deep Links for Navigation**
   - More reliable than UI-based navigation
   - Provides deterministic state

3. **Export Diagnostic Data Regularly**
   - After each test sequence
   - When debugging failures

4. **Monitor Performance Metrics**
   - Frame rate during interactions
   - Response time for taps
   - Memory usage trends

## Troubleshooting

### No Logs Appearing
- Verify app is running: `xcrun simctl listapps <device_id>`
- Check process name matches exactly
- Ensure diagnostic mode is enabled

### Calibration Points Not Registering
- Check diagnostic overlay is visible
- Verify tap coordinates in logs
- Ensure no modal dialogs are blocking

### Export Not Working
- Verify deep link scheme is registered
- Check app is in foreground
- Review console logs for errors