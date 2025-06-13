# MCP Server Usage with ArkavoReference App

## Important: Do NOT Use setup_xcuitest for Calibration

The `setup_xcuitest` tool is for a different purpose and will fail with connection errors. For calibration and testing with visual feedback, use the ArkavoReference app instead.

## Correct Workflow

### 1. Install ArkavoReference App

First, check if the ArkavoReference app is installed:

```json
{
  "tool": "app_diagnostic",
  "parameters": {
    "bundle_id": "com.arkavo.ArkavoReference"
  }
}
```

If not installed, use the calibration manager to build and install it:

```json
{
  "tool": "calibration_manager",
  "parameters": {
    "action": "install_reference_app",
    "device_id": "YOUR_DEVICE_ID"
  }
}
```

### 2. Start Log Streaming

Before launching the app, start log streaming to capture diagnostic events:

```json
{
  "tool": "log_stream",
  "parameters": {
    "action": "start",
    "process_name": "ArkavoReference"
  }
}
```

### 3. Launch in Calibration Mode

Use the calibration setup tool or deep links:

**Option A - Using setup_calibration:**
```json
{
  "tool": "setup_calibration",
  "parameters": {}
}
```

**Option B - Using deep links directly:**
```json
{
  "tool": "deep_link",
  "parameters": {
    "url": "arkavo-edge://calibration",
    "bundle_id": "com.arkavo.ArkavoReference"
  }
}
```

### 4. Run Calibration

Start the calibration process:

```json
{
  "tool": "calibration_manager",
  "parameters": {
    "action": "start_calibration",
    "device_id": "YOUR_DEVICE_ID"
  }
}
```

### 5. Monitor Progress

Check calibration status:

```json
{
  "tool": "calibration_manager",
  "parameters": {
    "action": "get_status",
    "device_id": "YOUR_DEVICE_ID",
    "session_id": "SESSION_ID_FROM_START"
  }
}
```

### 6. Export Diagnostic Data

Trigger diagnostic export from the app:

```json
{
  "tool": "app_diagnostic_export",
  "parameters": {
    "bundle_id": "com.arkavo.ArkavoReference"
  }
}
```

### 7. Read Logs

View captured logs:

```json
{
  "tool": "log_stream",
  "parameters": {
    "action": "read",
    "limit": 50
  }
}
```

## Key Points

1. **ArkavoReference app** is the correct app for calibration with visual feedback
2. **DO NOT use setup_xcuitest** - it's for a different test host app
3. **Bundle ID**: `com.arkavo.ArkavoReference`
4. **Deep link schemes**: 
   - `arkavo-edge://calibration` - Opens calibration screen
   - `arkavo-reference://diagnostic/export` - Exports diagnostic data

## Features of ArkavoReference App

- **Diagnostic Mode**: Enabled by default in debug builds
- **Visual Overlay**: Shows grid, tap indicators, and coordinates
- **Event Logging**: All interactions logged with timestamps
- **Deep Link Navigation**: Direct access to test screens
- **Export Capability**: JSON export of all diagnostic data

## URL Confirmation Dialogs

iOS shows a system confirmation dialog when opening URL schemes. The MCP server now handles this automatically by:
1. Waiting 1.5 seconds for the dialog to appear
2. Tapping the "Open" button at coordinates (195, 490)

You can also manually handle URL dialogs using:
```json
{
  "tool": "url_dialog",
  "parameters": {
    "action": "tap_open"
  }
}
```

## Troubleshooting

### "Reference app not installed"
- Build and install the ArkavoReference app from the iOS project
- Ensure you're using the correct bundle ID

### "Deep link failed"
- The app must be launched first before opening deep links
- Check that the app is in the foreground

### "Calibration stuck in initializing"
- Check that ArkavoReference app is installed
- Verify the app launched successfully
- Review logs for any errors

### "URL dialog not handled"
- The dialog might appear at different coordinates on some devices
- Use the `url_dialog` tool with appropriate device-specific coordinates
- Take a screenshot to verify dialog presence