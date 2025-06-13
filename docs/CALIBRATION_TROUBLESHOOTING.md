# Calibration Troubleshooting Guide

## Common Issues

### 1. "Failed to open calibration mode" Error

This error typically occurs when the test host app doesn't have the URL scheme registered properly.

**Solution:**
1. Run `setup_xcuitest` with `force_reinstall: true` to ensure a fresh installation:
```json
{
  "tool": "setup_xcuitest",
  "arguments": {
    "force_reinstall": true
  }
}
```

2. After installation completes, try calibration again:
```json
{
  "tool": "calibration_manager",
  "arguments": {
    "action": "start_calibration"
  }
}
```

### 2. "Test host app not installed" Error

The calibration system requires the ArkavoTestHost app to be installed.

**Solution:**
Run `setup_xcuitest` first:
```json
{
  "tool": "setup_xcuitest",
  "arguments": {}
}
```

### 3. Deep Link Prompts Blocking Automation

Starting with iOS 14+, deep links may prompt for user confirmation ("Open in ArkavoTestHost?") which blocks automation.

**Solution:**
The calibration system now uses environment variables instead of deep links to avoid this issue. The app will automatically enter calibration mode when launched with the `ARKAVO_CALIBRATION_MODE=1` environment variable.

**Note:** If you're still seeing deep link prompts, ensure you're using the latest version of the MCP server which includes this fix.

### 4. Basic Calibration Fallback

If visual calibration mode fails, the system will continue with basic coordinate mapping. This still provides:
- Device profile detection
- Screen size mapping
- Basic tap verification

However, you won't get:
- Visual coordinate display
- Screenshot-based verification
- Tap markers on screen

## Alternative: Manual Calibration

If automated calibration continues to fail, you can use the `setup_calibration` tool directly:

```json
{
  "tool": "setup_calibration",
  "arguments": {}
}
```

This will attempt to launch just the calibration UI without the full automated process.

## Debugging Tips

1. **Check Test Host Status:**
```json
{
  "tool": "xctest_status",
  "arguments": {}
}
```

2. **Verify App Installation:**
```json
{
  "tool": "app_management",
  "arguments": {
    "action": "list_apps"
  }
}
```

3. **View Simulator Logs:**
Look for errors related to URL scheme registration or app launch failures.

4. **Try Direct Launch:**
```json
{
  "tool": "app_launcher",
  "arguments": {
    "action": "launch",
    "bundle_id": "com.arkavo.testhost"
  }
}
```

Then manually launch in calibration mode:
```bash
SIMCTL_CHILD_ARKAVO_CALIBRATION_MODE=1 xcrun simctl launch [device_id] com.arkavo.testhost
```