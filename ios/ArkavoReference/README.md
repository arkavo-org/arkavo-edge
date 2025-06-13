# Arkavo Reference iOS App

A deterministic iOS reference application designed for testing and calibrating UI automation tools, particularly the Arkavo MCP server.

## Purpose

This app serves as:
1. **Ground Truth** - Fixed, known UI elements for validating automation tools
2. **Calibration Platform** - Deterministic layouts for tuning coordinate systems
3. **Test Harness** - Comprehensive UI element types for testing all automation scenarios
4. **WWDC Demo** - Showcase authentication flows and biometric testing capabilities

## Features

### Core Functionality
- **Authentication Flow** - Login, registration, biometric authentication
- **UI Element Testing** - Checkboxes, buttons, forms, switches, sliders
- **Coordinate System** - Visual grid overlay with percentage-based positioning
- **Diagnostic Mode** - Real-time event logging, tap indicators, coordinate display
- **Deep Linking** - Navigate directly to test screens via URLs
- **Biometric Scenarios** - Test all Face ID/Touch ID states and edge cases

### Test Screens

1. **Authentication**
   - Login with username/password
   - Face ID/Touch ID authentication
   - Registration flow with terms acceptance
   - Account lockout after failed attempts

2. **Checkbox Tests** (`arkavo-reference://screen/checkboxes`)
   - Standard checkboxes in grid layout
   - Master/select all functionality
   - Disabled states
   - Hidden checkboxes requiring scroll
   - Custom styled checkboxes

3. **Biometric Tests** (`arkavo-reference://screen/biometric`)
   - Success/failure scenarios
   - User cancellation
   - Not enrolled states
   - Lockout simulation
   - Passcode fallback

4. **Form Elements** (`arkavo-reference://screen/forms`)
   - Text fields, secure fields
   - Date pickers, option pickers
   - Text editors
   - Form validation

5. **Interactions** (`arkavo-reference://screen/interactions`)
   - Tap, double-tap, long press
   - Drag and drop
   - Swipe gestures
   - Coordinate tracking

6. **Grid & Edges** (`arkavo-reference://screen/grid`)
   - 10x10 grid with percentage labels
   - Edge markers (corners, centers)
   - Safe area indicators
   - Device bounds visualization

### Diagnostic Features

- **Event Logging** - All interactions logged with timestamps
- **Tap Visualization** - Shows tap location with coordinates
- **Grid Overlay** - 10% increment grid for positioning
- **Coordinate Display** - Both absolute and normalized coordinates
- **Safe Area Markers** - Visual indicators for device safe areas

### Accessibility

Every element includes:
- `accessibilityIdentifier` - Unique, stable identifier for automation
- `accessibilityLabel` - Descriptive label for screen readers
- `accessibilityValue` - Current state (checked/unchecked, etc.)
- `accessibilityHint` - Usage instructions

## Building & Running

### Requirements
- Xcode 15.0+
- iOS 16.0+
- Swift 5.9+

### Build Steps
```bash
cd ios/ArkavoReference
open ArkavoReference.xcodeproj
# Build and run on simulator
```

### Simulator Setup
1. Launch iOS Simulator
2. Enable Face ID: Device > Face ID > Enrolled
3. Build and run the app

## Deep Links

Navigate directly to test screens:
```
arkavo-reference://screen/checkboxes
arkavo-reference://screen/biometric
arkavo-reference://screen/forms
arkavo-reference://screen/interactions
arkavo-reference://screen/grid

# Diagnostic controls
arkavo-reference://diagnostic/enable
arkavo-reference://diagnostic/disable
arkavo-reference://diagnostic/clear

# Automated tests
arkavo-reference://test/checkbox_sequence
arkavo-reference://test/biometric_flow
arkavo-reference://test/form_validation
```

## Using with Arkavo MCP

The app is designed to work with the Arkavo MCP server for automated testing:

1. **Known Elements** - All elements have stable identifiers
2. **Predictable Layout** - Elements never change position
3. **State Verification** - Visual feedback for all state changes
4. **Error Scenarios** - Test failure paths and edge cases

## Element Identifiers

Key identifiers for automation:

### Authentication
- `username_field`
- `password_field`
- `sign_in_button`
- `face_id_button`
- `create_account_button`

### Checkboxes
- `master_checkbox`
- `checkbox_0` through `checkbox_19`
- `disabled_checkbox_checked`
- `disabled_checkbox_unchecked`

### Biometric
- `test_biometric_success_button`
- `test_biometric_failure_button`
- `enrollment_alert`

## Testing Patterns

### Checkbox Testing
```swift
// Tap master checkbox
tap(identifier: "master_checkbox")

// Verify all checkboxes checked
for i in 0..<8 {
    assert(isChecked("checkbox_\(i)"))
}
```

### Biometric Testing
```swift
// Trigger biometric prompt
tap(identifier: "test_biometric_success_button")

// Handle Face ID dialog
// Success: Simulator > Device > Face ID > Matching Face
// Failure: Simulator > Device > Face ID > Non-matching Face
```

### Coordinate Testing
```swift
// Tap at 50%, 50% (center)
tap(x: 0.5, y: 0.5)

// Tap at specific grid intersection
tap(identifier: "grid_label_5_5") // 50%, 50%
```

## Debugging

1. **Enable Diagnostics**: `arkavo-reference://diagnostic/enable`
2. **View Event Log**: Bottom of screen shows recent events
3. **Export Data**: `arkavo-reference://diagnostic/export`
4. **Clear Log**: `arkavo-reference://diagnostic/clear`

## Architecture

- **SwiftUI** - Modern declarative UI
- **Combine** - Reactive state management
- **LocalAuthentication** - Biometric authentication
- **Diagnostic Manager** - Centralized event logging
- **Navigation Manager** - Deep link handling

## Contributing

When adding new test elements:
1. Use unique `accessibilityIdentifier`
2. Log state changes to diagnostic manager
3. Provide visual feedback for interactions
4. Document the identifier in this README
5. Keep layouts deterministic (no random positioning)