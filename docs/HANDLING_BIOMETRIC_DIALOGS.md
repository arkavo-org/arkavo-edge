# Handling Biometric Dialogs in iOS Simulator

## Quick Solution

When a biometric dialog appears and blocks your test, use:

```json
{
  "tool": "biometric_auth",
  "arguments": {
    "action": "cancel"
  }
}
```

This will:
1. Send a biometric cancel command
2. Send ESC key to dismiss the dialog

## Alternative Solutions

### 1. Using biometric_dialog_handler (no external tools)
```json
{
  "tool": "biometric_dialog_handler",
  "arguments": {
    "action": "dismiss"
  }
}
```

### 2. Direct keyboard command
```bash
xcrun simctl io <device-id> sendkey escape
```

### 3. Accept the biometric auth
```json
{
  "tool": "biometric_auth",
  "arguments": {
    "action": "match"
  }
}
```

## Why Taps May Fail

The error shows that coordinate-based taps are failing because:
1. The `instruments` tool is deprecated/removed in newer Xcode versions
2. Standard `xcrun simctl` doesn't have a built-in tap command
3. External tools like `fbsimctl` or `applesimutils` are not installed

## Best Practice

For biometric dialogs, always use the dedicated biometric tools rather than trying to tap buttons:
- `biometric_auth` with action "cancel" - Most reliable
- `biometric_dialog_handler` with action "dismiss" - Alternative approach
- Keyboard shortcuts (ESC) - Direct and simple