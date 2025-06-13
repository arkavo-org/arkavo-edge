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
                            "enum": ["overview", "text_based_tapping", "workflows", "debugging", "examples", "calibration"],
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
            "overview" => {
                r#"
# iOS Automation with Coordinate-Based Tapping

This MCP server provides iOS UI automation with COORDINATE-BASED TAPPING as the PRIMARY method.

## ⚠️ IMPORTANT: Calibration Setup

For calibration with visual feedback:
- Simply run `calibration_manager` with action `start_calibration`
- The ArkavoReference app will be installed automatically if needed

**DO NOT use setup_xcuitest** - it's for a different purpose and will fail with connection errors.

## 🎯 RECOMMENDED APPROACH: Coordinate Tapping

**NO SETUP REQUIRED!** Just use coordinates from screenshots:

1. **Take a screenshot**:
   ```json
   {
     "tool": "screen_capture",
     "arguments": {
       "name": "current_screen"
     }
   }
   ```

2. **Read the image to see UI elements**:
   ```json
   {
     "tool": "Read",
     "arguments": {
       "file_path": "test_results/current_screen.png"
     }
   }
   ```

3. **Tap using coordinates**:
   ```json
   {
     "tool": "ui_interaction",
     "arguments": {
       "action": "tap",
       "target": {"x": 200, "y": 400}
     }
   }
   ```

## ✅ Why Coordinates Are Better:

- **Works immediately** - No setup or initialization needed
- **More reliable** - No connection timeouts or setup failures
- **Faster execution** - Direct tapping via embedded idb_companion
- **Always available** - Works with any UI element

## ⚠️ Avoid Text-Based Tapping:

Text-based tapping requires setup_xcuitest which:
- Often fails with timeouts
- Requires complex initialization
- Is slower and less reliable
- Should only be used as absolute last resort

## Quick Start

1. Use device_management to get device_id
2. (Optional) Run calibration_manager for better accuracy
3. Use screen_capture to see the UI
4. Read the screenshot image
5. Identify element positions
6. Use ui_interaction with coordinates

Example:
```json
{"action": "tap", "target": {"x": 200, "y": 400}}
```
is MUCH better than:
```json
{"action": "tap", "target": {"text": "Sign In"}}
```
"#
            }
            "text_based_tapping" => {
                r#"
# Text-Based Tapping with XCUITest

## Prerequisites
⚠️ **MUST run setup_xcuitest first!** Text-based tapping will NOT work without XCUITest initialization.

## How It Works
When XCUITest is set up, you can find elements by their visible text:

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
"#
            }
            "workflows" => {
                r#"
# Common Automation Workflows

## 🎯 RECOMMENDED: Coordinate-Based Workflow

1. **Get device ID**: 
   ```json
   {"tool": "device_management", "arguments": {"action": "list"}}
   ```

2. **Start tapping immediately** (NO SETUP NEEDED!):
   ```json
   {"tool": "screen_capture", "arguments": {"name": "screen"}}
   {"tool": "Read", "arguments": {"file_path": "test_results/screen.png"}}
   {"tool": "ui_interaction", "arguments": {"action": "tap", "target": {"x": 200, "y": 400}}}
   ```

## Login Flow (Using Coordinates)
1. screen_capture {"name": "login_screen"}  
2. Read the image to identify element positions
3. Tap email field: {"action": "tap", "target": {"x": 200, "y": 300}}
4. Clear and type: {"action": "clear_text"}, then {"action": "type_text", "value": "user@example.com"}
5. Tap password field: {"action": "tap", "target": {"x": 200, "y": 400}}  
6. Clear and type: {"action": "clear_text"}, then {"action": "type_text", "value": "password123"}
7. Submit: {"action": "tap", "target": {"x": 200, "y": 500}}

## Text Input Rules
- ALWAYS tap the field first (using coordinates)
- Use clear_text to remove existing content
- Then use type_text with your new value

## Navigation
- Take screenshot first
- Identify button/link positions visually
- Tap using coordinates

## Form Filling
- Screenshot to see all fields
- For each field: tap by coordinates → clear → type
- Submit using button coordinates

## ⚠️ AVOID setup_xcuitest
- It often fails with timeouts
- Coordinates work immediately without any setup
- Only use text-based tapping as absolute last resort
"#
            }
            "debugging" => {
                r#"
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
"#
            }
            "log_streaming" => {
                r#"
# Log Streaming and Diagnostics

## Real-Time Log Streaming

Stream logs from iOS apps to debug issues and monitor behavior:

**Start streaming:**
```json
{
  "tool": "log_stream",
  "parameters": {
    "action": "start",
    "process_name": "ArkavoReference"
  }
}
```

**Read recent logs:**
```json
{
  "tool": "log_stream",
  "parameters": {
    "action": "read",
    "limit": 50
  }
}
```

**Stop streaming:**
```json
{
  "tool": "log_stream",
  "parameters": {
    "action": "stop",
    "stream_id": "<stream_id>"
  }
}
```

## ArkavoReference Diagnostic Export

Export diagnostic data from the reference app:

```json
{
  "tool": "app_diagnostic_export",
  "parameters": {
    "bundle_id": "com.arkavo.ArkavoReference"
  }
}
```

This triggers the app to export:
- Tap event history with coordinates
- UI state changes
- Performance metrics
- Device information

## Calibration with Feedback

1. **Start log stream** - Capture all diagnostic events
2. **Launch in calibration mode** - `arkavo-edge://calibration`
3. **Run calibration** - Monitor tap accuracy in logs
4. **Export diagnostics** - Get complete interaction history
5. **Analyze results** - Verify calibration accuracy

## Tips
- Always start log stream before launching app
- Use process name exactly as shown in Activity Monitor
- Diagnostic data appears in logs after export
- Filter logs with custom predicates for specific events
"#
            }
            "examples" => {
                r#"
# Complete Examples

## 🎯 RECOMMENDED: Coordinate-Based Testing

```python
# NO SETUP NEEDED! Start testing immediately!

## Login Test (Using Coordinates)
# 1. See what's on screen
tool("screen_capture", {"name": "login"})
tool("Read", {"file_path": "test_results/login.png"})

# 2. Fill email (you see field at position 200,300)
tool("ui_interaction", {"action": "tap", "target": {"x": 200, "y": 300}})
tool("ui_interaction", {"action": "clear_text"})
tool("ui_interaction", {"action": "type_text", "value": "test@example.com"})

# 3. Fill password (you see field at position 200,400)
tool("ui_interaction", {"action": "tap", "target": {"x": 200, "y": 400}})
tool("ui_interaction", {"action": "clear_text"})
tool("ui_interaction", {"action": "type_text", "value": "secret123"})

# 4. Submit (you see button at position 200,500)
tool("ui_interaction", {"action": "tap", "target": {"x": 200, "y": 500}})

# 5. Verify success
tool("screen_capture", {"name": "after_login"})
```

## Settings Navigation
```python
# Find settings button position
tool("screen_capture", {"name": "main_screen"})
tool("Read", {"file_path": "test_results/main_screen.png"})
# You see Settings at position (350, 800)
tool("ui_interaction", {"action": "tap", "target": {"x": 350, "y": 800}})
```

## Scrolling to Find Elements
```python
# If element not visible
tool("ui_interaction", {"action": "scroll", "direction": "down", "amount": 5})
tool("screen_capture", {"name": "after_scroll"})
tool("Read", {"file_path": "test_results/after_scroll.png"})
# Now tap element at its coordinates
tool("ui_interaction", {"action": "tap", "target": {"x": 200, "y": 600}})
```

## ⚠️ Why NOT to use text-based tapping:
- Requires setup_xcuitest which often fails
- Has connection timeouts
- Slower than coordinates
- Less reliable

ALWAYS use coordinates from screenshots instead!
"#
            }
            "calibration" => {
                r#"
# Calibration System for iOS UI Automation

The calibration system helps ensure accurate tap coordinates by displaying actual tap locations on screen for easy verification.

## 🎯 Visual Calibration Method (Recommended)

### setup_calibration
Launches the test host app in calibration mode with visual coordinate display:

```json
{
  "tool": "setup_calibration",
  "arguments": {}
}
```

Features:
- **Large coordinate display**: Shows "X: 123 Y: 456" prominently on screen
- **Visual markers**: Green circles mark each tap location
- **Persistent display**: All tap markers remain visible
- **Screenshot-friendly**: High contrast for easy OCR/detection

### How to Use Visual Calibration:

1. **Launch calibration mode**:
```json
{"tool": "setup_calibration", "arguments": {}}
```

2. **Tap and capture coordinates**:
```json
// Tap at a location
{"tool": "ui_interaction", "arguments": {"action": "tap", "target": {"x": 195, "y": 422}}}

// Take screenshot to see displayed coordinates
{"tool": "ui_query", "arguments": {"action": "screenshot"}}
```

3. **Compare results**:
- If displayed coordinates match sent coordinates: Perfect calibration!
- If offset exists: Apply inverse offset to future taps

## 📊 Automated Calibration Manager

### calibration_manager
Automates calibration with intelligent offset detection:

- `start_calibration` - Begin automated calibration (installs ArkavoReference app automatically if needed)
- `get_status` - Check calibration progress
- `get_calibration` - Retrieve calibration data
- `list_devices` - Show all calibrated devices
- `enable_monitoring` - Auto-recalibration when needed
- `install_reference_app` - Manually install ArkavoReference app (usually not needed)

### Quick Start:

```json
// Simply start calibration - app will be installed automatically if needed
{
  "tool": "calibration_manager",
  "arguments": {
    "action": "start_calibration",
    "device_id": "YOUR_DEVICE_ID"
  }
}

// Monitor progress
{
  "tool": "calibration_manager",
  "arguments": {
    "action": "get_status",
    "session_id": "SESSION_ID_FROM_START"
  }
}
```

**IMPORTANT**: Do NOT use setup_xcuitest - it's for a different purpose and will fail

### URL Dialog Handling

iOS shows a system dialog when opening deep links. The calibration process handles this automatically, but you can also use:

```json
{
  "tool": "url_dialog",
  "arguments": {
    "action": "tap_open"
  }
}
```

## When to Calibrate

- First time using a new device/simulator
- After iOS updates
- If taps are missing their targets
- When switching between device types

## Benefits

✅ **Visual verification** - See exactly where taps land
✅ **Easy debugging** - Screenshot shows coordinates
✅ **No guesswork** - Precise offset calculation
✅ **Integrated solution** - Built into test host app

## Best Practices

- Use visual calibration for manual verification
- Use automated calibration for production
- Save screenshots for debugging
- Recalibrate after major changes
"#
            }
            _ => {
                "Unknown topic. Available topics: overview, text_based_tapping, workflows, debugging, examples, calibration, log_streaming"
            }
        };

        Ok(serde_json::json!({
            "topic": topic,
            "content": content,
            "tips": [
                "🎯 ALWAYS use coordinate-based taps - they work immediately!",
                "📸 Take screenshots and read them to identify element positions",
                "⚠️ AVOID setup_xcuitest - it often fails with timeouts",
                "✅ Coordinates are more reliable than text-based tapping",
                "💡 No setup needed - just screenshot → read → tap coordinates"
            ]
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
