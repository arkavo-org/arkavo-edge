# Biometric Dialog Handling

This document describes how to handle biometric authentication dialogs in iOS simulators without requiring external tools.

## Overview

When testing iOS apps that use biometric authentication (Face ID/Touch ID), dialogs can block test automation. We provide several tools to handle these dialogs:

1. **biometric_dialog_handler** - Uses built-in simulator commands (no external dependencies)
2. **accessibility_dialog_handler** - Alternative approach using accessibility features

## Using biometric_dialog_handler

This is the recommended approach as it requires no external tools:

### Dismiss Dialog
```json
{
  "tool": "biometric_dialog_handler",
  "arguments": {
    "action": "dismiss"
  }
}
```

### Cancel Dialog
```json
{
  "tool": "biometric_dialog_handler",
  "arguments": {
    "action": "cancel"
  }
}
```

### Accept Biometric Auth
```json
{
  "tool": "biometric_dialog_handler",
  "arguments": {
    "action": "accept"
  }
}
```

### Use Passcode Instead
```json
{
  "tool": "biometric_dialog_handler",
  "arguments": {
    "action": "use_passcode",
    "passcode": "1234"  // Optional, defaults to "1234"
  }
}
```

## How It Works

The handler uses standard `xcrun simctl` commands:
- **dismiss/cancel**: Sends ESC key or HOME button press
- **accept**: Uses `xcrun simctl ui biometric match` to simulate successful auth
- **use_passcode**: Types the passcode digits and presses return

## Fallback Options

If the dialog persists, you can:

1. Use the original `biometric_auth` tool with action "cancel"
2. Try `system_dialog` tool to handle it as a generic dialog
3. Use keyboard shortcuts via `xcrun simctl io sendkey`

## Coordinate-Based Approach

If you know the button positions, you can use direct taps:

```json
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {
      "x": 196,  // Center of screen
      "y": 500   // Typical cancel button position
    }
  }
}
```

## No External Dependencies

These handlers work with just Xcode installed, making them accessible to all users without additional setup.