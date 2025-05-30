# Face ID Authentication in iOS Simulator

## Overview

When testing apps that use Face ID, you need to:
1. Enroll Face ID in the simulator
2. Provide a matching face when prompted

This matches the simulator menu options shown in Device > Face ID.

## Quick Start - Enable Face ID and Authenticate

### Step 1: Enroll Face ID
```json
{
  "tool": "face_id_control",
  "arguments": {
    "action": "enroll"
  }
}
```

### Step 2: When Face ID Dialog Appears, Provide Matching Face
```json
{
  "tool": "face_id_control",
  "arguments": {
    "action": "match"
  }
}
```

Or use the original biometric tool:
```json
{
  "tool": "biometric_auth",
  "arguments": {
    "action": "match"
  }
}
```

## Complete Face ID Testing Flow

1. **Check Current Status**
```json
{
  "tool": "face_id_status",
  "arguments": {}
}
```

2. **Enroll Face ID** (if not already enrolled)
```json
{
  "tool": "face_id_control",
  "arguments": {
    "action": "enroll"
  }
}
```

3. **Trigger Face ID in Your App** (tap login button, etc.)

4. **Provide Matching Face** when dialog appears
```json
{
  "tool": "face_id_control",
  "arguments": {
    "action": "match"
  }
}
```

## Available Actions

- `enroll` - Enable Face ID (like selecting "Enrolled" in simulator menu)
- `unenroll` - Disable Face ID 
- `match` - Simulate successful Face ID scan (like selecting "Matching Face")
- `no_match` - Simulate failed Face ID scan (like selecting "Non-matching Face")

## Why Not Cancel?

The biometric dialog appears because your app is requesting Face ID authentication. Instead of cancelling (which would fail the authentication), you should:
1. Ensure Face ID is enrolled
2. Provide a matching face to successfully authenticate

This allows your test to proceed through the normal authentication flow.