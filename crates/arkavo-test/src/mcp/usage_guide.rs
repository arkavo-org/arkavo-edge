use super::server::{Tool, ToolSchema};
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct UsageGuideKit {
    schema: ToolSchema,
}

impl UsageGuideKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "usage_guide".to_string(),
                description: "Get usage guidance and best practices for iOS automation with this MCP server. Returns helpful information about XCUITest capabilities, text-based interactions, and automation workflows.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "topic": {
                            "type": "string",
                            "enum": ["overview", "text_based_tapping", "workflows", "debugging", "examples"],
                            "description": "Specific topic to get guidance on"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for UsageGuideKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UsageGuideKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let topic = params
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("overview");

        let content = match topic {
            "overview" => r#"
# iOS Automation with XCUITest

This MCP server now uses XCUITest for reliable UI automation. Key improvements:

- **Text-based element finding** - Find buttons/fields by their visible text
- **Accessibility ID support** - Most reliable for test automation  
- **10-second timeout** - Automatically waits for elements to appear
- **Honest error messages** - Know exactly why something failed

## Quick Start

1. Always start with screen_capture to see the UI
2. Use text-based taps when you see button/link text
3. Only use coordinates as a last resort

Example:
```json
{"action": "tap", "target": {"text": "Sign In"}}
```
is MUCH better than:
```json
{"action": "tap", "target": {"x": 200, "y": 400}}
```
"#,
            "text_based_tapping" => r#"
# Text-Based Tapping with XCUITest

When you can see text in the UI, use it for reliable interaction:

## Button Tapping
```json
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {"text": "Sign In"}
  }
}
```

## Field Selection by Label
```json
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {"text": "Email"}
  }
}
```

## Accessibility ID (Most Reliable)
```json
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {"accessibility_id": "login_button"}
  }
}
```

XCUITest will search for elements matching your text/ID for up to 10 seconds.
"#,
            "workflows" => r#"
# Common Automation Workflows

## Login Flow
1. screen_capture {"name": "login_screen"}
2. Read the image to see UI elements
3. Tap email field: {"action": "tap", "target": {"text": "Email"}}
4. Clear and type: {"action": "clear_text"}, then {"action": "type_text", "value": "user@example.com"}
5. Tap password field: {"action": "tap", "target": {"text": "Password"}}  
6. Clear and type: {"action": "clear_text"}, then {"action": "type_text", "value": "password123"}
7. Submit: {"action": "tap", "target": {"text": "Sign In"}}

## Text Input Rules
- ALWAYS tap the field first
- Use clear_text to remove existing content
- Then use type_text with your new value

## Navigation
- Take screenshot first
- Look for tab labels, menu items, or navigation links
- Tap using the visible text

## Form Filling
- Screenshot to see all fields
- For each field: tap by label → clear → type
- Submit using button text
"#,
            "debugging" => r#"
# Debugging Tips

## If Text-Based Tap Fails

1. **Check exact text** - It's case-sensitive
2. **Try partial matches** - Sometimes full text has extra spaces
3. **Look for accessibility IDs** - Ask developers to add them
4. **Use coordinates as last resort** - Get from analyze_layout

## Common Issues

**"Element not found"**
- Text might be slightly different than what you see
- Element might not be tappable (decorative text)
- Try waiting or taking another screenshot

**"XCUITest not connected"**  
- Falls back to AppleScript automatically
- Still works but less reliable
- Check device is booted

**Text input not working**
- Did you tap the field first?
- Is the keyboard showing?
- Try clear_text before typing

## Best Practices
- Take screenshots between actions
- Verify UI state before interacting
- Read error messages - they're helpful!
- Use text/accessibility_id over coordinates
"#,
            "examples" => r#"
# Complete Examples

## Login Test
```python
# 1. See what's on screen
tool("screen_capture", {"name": "login"})
tool("read", {"file_path": "test_results/login.png"})

# 2. Fill email (you saw "Email" label)
tool("ui_interaction", {"action": "tap", "target": {"text": "Email"}})
tool("ui_interaction", {"action": "clear_text"})
tool("ui_interaction", {"action": "type_text", "value": "test@example.com"})

# 3. Fill password  
tool("ui_interaction", {"action": "tap", "target": {"text": "Password"}})
tool("ui_interaction", {"action": "clear_text"})
tool("ui_interaction", {"action": "type_text", "value": "secret123"})

# 4. Submit
tool("ui_interaction", {"action": "tap", "target": {"text": "Sign In"}})

# 5. Verify success
tool("screen_capture", {"name": "after_login"})
```

## Settings Navigation
```python
# Find settings
tool("screen_capture", {"name": "main_screen"})
tool("ui_interaction", {"action": "tap", "target": {"text": "Settings"}})

# Or use tab bar
tool("ui_interaction", {"action": "tap", "target": {"text": "Profile"}})
```

## Scrolling to Find Elements
```python
# If element not visible
tool("ui_interaction", {"action": "scroll", "direction": "down", "amount": 5})
tool("screen_capture", {"name": "after_scroll"})
# Now try tapping
tool("ui_interaction", {"action": "tap", "target": {"text": "Advanced Settings"}})
```
"#,
            _ => "Unknown topic. Available topics: overview, text_based_tapping, workflows, debugging, examples"
        };

        Ok(serde_json::json!({
            "topic": topic,
            "content": content,
            "tips": [
                "Always prefer text-based taps over coordinates",
                "Take screenshots to see current UI state",
                "Read error messages - they contain helpful info",
                "XCUITest waits 10 seconds for elements automatically"
            ]
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}