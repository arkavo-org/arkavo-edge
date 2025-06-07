# UI Element Handling Guide

## Overview

The `ui_element_handler` tool provides advanced strategies for interacting with UI elements in iOS simulators, particularly for challenging elements like checkboxes, switches, and buttons that may not respond to standard taps.

## When to Use

Use this tool when:
- Standard `ui_interaction` tap fails
- Checkboxes won't toggle
- Switches don't respond
- Elements require special interaction patterns

## Available Actions

### 1. tap_checkbox
Tries multiple strategies to tap a checkbox:
- Direct tap
- Left offset tap (for labels)
- Double tap
- Directional offsets

```json
{
  "tool": "ui_element_handler",
  "arguments": {
    "action": "tap_checkbox",
    "coordinates": {
      "x": 51,
      "y": 720
    }
  }
}
```

### 2. tap_with_retry
Attempts to tap an element multiple times with delays:

```json
{
  "tool": "ui_element_handler",
  "arguments": {
    "action": "tap_with_retry",
    "coordinates": {
      "x": 100,
      "y": 200
    },
    "retry_count": 3
  }
}
```

### 3. double_tap
Performs two quick taps (some UI frameworks require this):

```json
{
  "tool": "ui_element_handler",
  "arguments": {
    "action": "double_tap",
    "coordinates": {
      "x": 150,
      "y": 250
    }
  }
}
```

### 4. long_press
Performs a long press gesture:

```json
{
  "tool": "ui_element_handler",
  "arguments": {
    "action": "long_press",
    "coordinates": {
      "x": 200,
      "y": 300
    }
  }
}
```

### 5. tap_switch
Taps with right offset for toggle switches:

```json
{
  "tool": "ui_element_handler",
  "arguments": {
    "action": "tap_switch",
    "coordinates": {
      "x": 50,
      "y": 400
    }
  }
}
```

## Best Practices

1. **Try Standard First**: Always try `ui_interaction` first
2. **Use Screen Analysis**: Take screenshots to verify element positions
3. **Check Response**: Take screenshot after interaction to verify success
4. **Adjust Coordinates**: If checkbox tap fails, try adjusting coordinates slightly

## Example Workflow

```javascript
// 1. Try standard tap
ui_interaction({ action: "tap", target: {x: 51, y: 720} })

// 2. If that fails, use checkbox handler
ui_element_handler({ action: "tap_checkbox", coordinates: {x: 51, y: 720} })

// 3. Take screenshot to verify
screen_capture({ name: "after_checkbox" })

// 4. If still unchecked, try with retry
ui_element_handler({ action: "tap_with_retry", coordinates: {x: 51, y: 720}, retry_count: 5 })
```

## Troubleshooting

- **Checkbox Won't Check**: Try `double_tap` or adjust coordinates by ±5 pixels
- **Switch Won't Toggle**: Use `tap_switch` which adds right offset
- **Nothing Works**: Element might be disabled or require scrolling into view first