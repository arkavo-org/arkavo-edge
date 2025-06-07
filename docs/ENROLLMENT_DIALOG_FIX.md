# Biometric Enrollment Dialog Fix

## Problem

The AI agent was stuck trying to tap the Cancel button on the "Simulator requires enrolled biometrics to use passkeys" dialog on iPhone 16 Pro Max. The agent attempted multiple coordinates but none were successful:

- (330, 1305) - adjusted to (330, 931) because it was out of bounds
- (330, 780) - tapped but didn't work
- (330, 820) - tapped but didn't work

## Root Cause

1. The existing `passkey_dialog_handler` was providing outdated coordinates that were designed for smaller devices (196.5, 550.0)
2. The iPhone 16 Pro Max has a logical resolution of 430x932, requiring different coordinates
3. The Cancel button on the enrollment dialog appears centered horizontally (x=215) and near the bottom (y≈830)

## Solution

### 1. Updated PasskeyDialogHandler

Updated the passkey dialog handler to provide device-specific coordinate guidance:
- iPhone 16 Pro Max (430x932): Cancel at (215, 830)
- iPhone 16 Pro (393x852): Cancel at (196.5, 750)
- iPhone 16 (390x844): Cancel at (195, 740)
- iPhone SE (375x667): Cancel at (187.5, 550)

### 2. Created EnrollmentDialogHandler

Created a new specialized tool `enrollment_dialog` that provides precise Cancel button coordinates based on device type:

```rust
pub struct EnrollmentDialogHandler {
    // Provides device-specific coordinates for the Cancel button
    // in the biometric enrollment dialog
}
```

Features:
- `get_cancel_coordinates` action: Returns exact coordinates for the Cancel button based on device type
- `tap_cancel` action: Provides guidance on how to tap the Cancel button with ui_interaction tool
- Device-specific coordinate mappings for all iPhone models

### 3. Enhanced Debugging

Added additional debug logging to the UI interaction system to track tap attempts and coordinate adjustments.

## Usage

When the agent encounters the enrollment dialog, it should:

1. Use the `enrollment_dialog` tool with `get_cancel_coordinates` action to get precise coordinates
2. Use the `ui_interaction` tool with the provided coordinates to tap the Cancel button

Example:
```json
// First get coordinates
{
  "tool": "enrollment_dialog",
  "params": {
    "action": "get_cancel_coordinates"
  }
}

// Then tap using the returned coordinates
{
  "tool": "ui_interaction", 
  "params": {
    "action": "tap",
    "target": {
      "x": 215,
      "y": 830
    }
  }
}
```

## Testing

Added comprehensive tests in `enrollment_dialog_test.rs` to verify the handler returns correct coordinates for different device types.

## Future Improvements

1. Consider using computer vision to detect dialog buttons dynamically
2. Implement a fallback mechanism that tries multiple coordinate sets
3. Add support for iPad and other device form factors