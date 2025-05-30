# Biometric Testing Guide for Agents

## Quick Decision Tree

When you encounter a biometric dialog, ask yourself:

1. **What am I testing?**
   - Login flow → Use `success_path` (authenticate successfully)
   - Error handling → Use `face_not_recognized` or `user_cancels`
   - Security → Use `biometric_lockout` or `fallback_to_passcode`
   - Just need to proceed → Use `success_path`

2. **Not sure what to do?**
   ```json
   {
     "tool": "biometric_test_scenario",
     "arguments": {
       "scenario": "check_current_state"
     }
   }
   ```

## Smart Handler (Recommended)

Let the tool decide based on your test type:

```json
{
  "tool": "smart_biometric_handler",
  "arguments": {
    "test_type": "login_flow"  // or "security_test", "edge_case_test", "ui_test"
  }
}
```

## Common Scenarios

### 1. Normal Login Test (Most Common)
**Goal**: Test the happy path where user successfully logs in
```json
{
  "tool": "biometric_test_scenario",
  "arguments": {
    "scenario": "success_path"
  }
}
```
**What it does**: Enrolls Face ID if needed, then provides matching face

### 2. Test Cancellation Handling
**Goal**: Verify app handles when user cancels biometric prompt
```json
{
  "tool": "biometric_test_scenario",
  "arguments": {
    "scenario": "user_cancels"
  }
}
```
**What it does**: Dismisses the dialog with ESC key

### 3. Test Failed Authentication
**Goal**: Verify app handles incorrect biometric
```json
{
  "tool": "biometric_test_scenario",
  "arguments": {
    "scenario": "face_not_recognized"
  }
}
```
**What it does**: Provides non-matching face

### 4. Test Biometric Lockout
**Goal**: Test what happens after multiple failures
```json
{
  "tool": "biometric_test_scenario",
  "arguments": {
    "scenario": "biometric_lockout"
  }
}
```
**What it does**: Fails 5 times to trigger lockout

### 5. Test Passcode Fallback
**Goal**: Test alternative authentication method
```json
{
  "tool": "biometric_test_scenario",
  "arguments": {
    "scenario": "fallback_to_passcode"
  }
}
```
**What it does**: Types default passcode (1234)

## Decision Matrix

| If you're testing... | Use scenario... | Expected outcome |
|---------------------|-----------------|------------------|
| Normal app flow | `success_path` | User logs in successfully |
| Error messages | `face_not_recognized` | App shows "Face not recognized" |
| Cancel button | `user_cancels` | App returns to login screen |
| Security features | `biometric_lockout` | App locks biometric, offers passcode |
| Alternative auth | `fallback_to_passcode` | User logs in with passcode |

## Best Practices

1. **Default to success**: If unsure, use `success_path` to proceed
2. **Test one scenario per test run**: Don't mix success and failure in one test
3. **Document your choice**: When using edge cases, explain why in your test

## Examples by Test Type

### Testing Login Flow
```
1. Navigate to login screen
2. Tap "Sign in with Face ID"
3. Use biometric_test_scenario with "success_path"
4. Verify user reaches home screen
```

### Testing Security
```
1. Navigate to login screen  
2. Tap "Sign in with Face ID"
3. Use biometric_test_scenario with "face_not_recognized"
4. Verify error message appears
5. Use biometric_test_scenario with "biometric_lockout" 
6. Verify passcode option appears
```

### Testing UI Elements
```
1. Use smart_biometric_handler with "ui_test"
2. (It will quickly authenticate you)
3. Continue testing other UI elements
```