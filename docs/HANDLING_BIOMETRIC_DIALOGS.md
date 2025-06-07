# Handling Biometric Dialogs in iOS Simulator

## Enrollment Dialog Handling

When encountering the "Simulator requires enrolled biometrics to use passkeys" dialog:

### Quick Solution - Complete Enrollment Flow
```json
{
  "tool": "enrollment_flow",
  "arguments": {
    "action": "complete_enrollment",
    "app_bundle_id": "com.arkavo.app"
  }
}
```

This will:
1. Dismiss the enrollment dialog
2. Enable Face ID enrollment in the simulator
3. Terminate and relaunch the app

### Alternative Solutions

#### Just Dismiss and Relaunch
```json
{
  "tool": "enrollment_flow",
  "arguments": {
    "action": "dismiss_and_relaunch"
  }
}
```

#### Basic Dialog Dismissal
```json
{
  "tool": "enrollment_dialog",
  "arguments": {
    "action": "handle_automatically"
  }
}
```

This will:
1. First try to dismiss the dialog using ESC key
2. If that fails, provide coordinates for manual tap

### Alternative Actions
```json
// Wait for dialog to appear
{
  "tool": "enrollment_dialog",
  "arguments": {
    "action": "wait_for_dialog"
  }
}

// Dismiss using keyboard
{
  "tool": "enrollment_dialog",
  "arguments": {
    "action": "dismiss"
  }
}

// Get cancel button coordinates
{
  "tool": "enrollment_dialog",
  "arguments": {
    "action": "get_cancel_coordinates"
  }
}
```

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