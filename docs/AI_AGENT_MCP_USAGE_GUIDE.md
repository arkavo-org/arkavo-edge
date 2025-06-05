# AI Agent MCP Usage Guide for iOS Automation

This guide helps AI agents use the Arkavo MCP server effectively for iOS automation.

## Key Improvements with XCUITest

The MCP server now uses XCUITest for reliable UI automation when possible. This provides:
- **Text-based element finding** - More reliable than coordinates
- **Accessibility ID support** - Most reliable for automation
- **10-second timeout** - Waits for elements to appear
- **Proper error messages** - Know exactly why something failed

## Best Practices for UI Automation

### 1. Always Start with Screenshots
```json
{"tool": "screen_capture", "arguments": {"name": "current_screen"}}
```
Then read the image to understand the UI:
```json
{"tool": "read", "arguments": {"file_path": "test_results/current_screen.png"}}
```

### 2. Prefer Text-Based Interactions
When you see text in the screenshot, use it for tapping:
```json
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {"text": "Sign In"}
  }
}
```

This is MUCH more reliable than:
```json
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {"x": 200, "y": 400}
  }
}
```

### 3. Use Accessibility IDs When Available
If developers have set accessibility identifiers:
```json
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {"accessibility_id": "login_button"}
  }
}
```

### 4. Text Input Workflow
Always tap the field first, then type:
```json
// Step 1: Tap the text field
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {"text": "Email"}  // or use coordinates if no label
  }
}

// Step 2: Clear existing text (if needed)
{
  "tool": "ui_interaction",
  "arguments": {"action": "clear_text"}
}

// Step 3: Type new text
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "type_text",
    "value": "user@example.com"
  }
}
```

### 5. Handle Biometric Dialogs Honestly
The biometric tools will return errors with helpful instructions:
```json
{
  "tool": "biometric_auth",
  "arguments": {"action": "match"}
}
```

Response will include manual steps if automation isn't possible.

### 6. Check Tool Responses
Always check if a tool succeeded before proceeding:
- Look for `"error"` in response - this indicates failure
- Don't assume success - verify with screenshots
- Read error messages - they contain helpful suggestions

## Common Patterns

### Login Flow
1. `screen_capture` - See login screen
2. `ui_interaction` tap with `{"text": "Email"}` or `{"text": "Username"}`
3. `ui_interaction` with `clear_text` then `type_text`
4. `ui_interaction` tap with `{"text": "Password"}`
5. `ui_interaction` with `clear_text` then `type_text`
6. `ui_interaction` tap with `{"text": "Sign In"}` or `{"text": "Login"}`

### Navigation
1. `screen_capture` - See current screen
2. Look for navigation elements (tabs, buttons, links)
3. `ui_interaction` tap with `{"text": "Settings"}` etc.

### Form Filling
1. `screen_capture` - See form
2. For each field:
   - Tap using field label text
   - Clear if needed
   - Type new value
3. Submit using button text

## What's Changed

### Old (Unreliable) Approach:
- Always used coordinates
- No element finding
- No wait for elements
- Fake success responses

### New (Reliable) Approach:
- XCUITest finds elements by text/ID
- Waits up to 10 seconds for elements
- Falls back gracefully
- Honest error responses

## Debugging Tips

1. If tap fails with text, try:
   - Different text variations (case sensitive)
   - Partial text matches
   - Accessibility IDs
   - Coordinates as last resort

2. If XCUITest isn't working:
   - Check device is booted
   - Look for initialization errors
   - Falls back to AppleScript automatically

3. For dynamic content:
   - Take screenshots between actions
   - Wait a moment after navigation
   - Verify UI state before interacting

## Example: Complete Login Test

```python
# 1. Capture initial screen
mcp.tool("screen_capture", {"name": "login_screen"})
mcp.tool("read", {"file_path": "test_results/login_screen.png"})

# 2. Enter email (assuming you see "Email" field)
mcp.tool("ui_interaction", {
    "action": "tap",
    "target": {"text": "Email"}
})
mcp.tool("ui_interaction", {"action": "clear_text"})
mcp.tool("ui_interaction", {
    "action": "type_text",
    "value": "test@example.com"
})

# 3. Enter password
mcp.tool("ui_interaction", {
    "action": "tap",
    "target": {"text": "Password"}
})
mcp.tool("ui_interaction", {"action": "clear_text"})
mcp.tool("ui_interaction", {
    "action": "type_text",
    "value": "secretpass123"
})

# 4. Submit
mcp.tool("ui_interaction", {
    "action": "tap",
    "target": {"text": "Sign In"}
})

# 5. Verify success
mcp.tool("screen_capture", {"name": "after_login"})
mcp.tool("read", {"file_path": "test_results/after_login.png"})
```

## Summary

The key change is that AI agents should now:
1. **Look for text labels** in screenshots
2. **Use text-based taps** instead of coordinates
3. **Trust error messages** - they're now accurate
4. **Follow the workflows** - they're based on what actually works

This will result in much more reliable iOS automation!