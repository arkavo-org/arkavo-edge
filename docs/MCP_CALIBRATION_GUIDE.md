# MCP Calibration Guide

This guide explains how to use the integrated calibration system in the MCP server for iOS UI automation.

## Overview

The calibration system helps ensure accurate tap coordinates by displaying the actual tap locations on screen. This is especially useful when dealing with coordinate offset issues between different simulators or iOS versions.

## How It Works

1. **Integrated App**: The calibration functionality is built into the ArkavoTestHost app, which is automatically installed when you run `setup_xcuitest`.

2. **Visual Feedback**: Each tap displays:
   - Large coordinate text in the center of the screen (e.g., "X: 195 Y: 422")
   - Green circular markers at tap locations
   - Small coordinate labels at each tap point
   - All markers remain visible for the entire session

3. **Screenshot Detection**: The coordinate display is designed for easy screenshot capture and OCR/detection by AI agents.

4. **Automatic Launch**: Calibration mode is launched via environment variable to avoid deep link prompts

## Setup Instructions

### 1. Install XCUITest (if not already done)
```json
{
  "tool": "setup_xcuitest",
  "arguments": {}
}
```

### 2. Launch Calibration Mode
```json
{
  "tool": "setup_calibration",
  "arguments": {}
}
```

The app will launch showing:
- "READY FOR TAP" in large text
- Instructions that it's waiting for automated taps

### 3. Perform Test Taps
Use the `ui_interaction` tool to tap at various screen locations:

```json
{
  "tool": "ui_interaction",
  "arguments": {
    "action": "tap",
    "target": {
      "x": 78,
      "y": 169
    }
  }
}
```

After each tap:
- The coordinate display updates to show "X: 78 Y: 169"
- A green marker appears at the tap location
- The display changes to green to indicate success

### 4. Take Screenshots
After each tap, use the screenshot tool to capture the displayed coordinates:

```json
{
  "tool": "ui_query",
  "arguments": {
    "action": "screenshot"
  }
}
```

### 5. Analyze Results
- Compare the coordinates you sent vs. what appears on screen
- If there's an offset, apply it to future taps
- The calibration automatically saves results after each tap

## Features

- **Real-time Feedback**: Coordinates display immediately upon tap
- **Persistent Markers**: All tap locations remain visible
- **High Contrast**: Black text on white/green background for easy OCR
- **Automatic Completion**: After 5 taps, calibration completes
- **File Output**: Results saved to the app's documents directory

## Example Calibration Sequence

```json
// 1. Setup calibration
{"tool": "setup_calibration", "arguments": {}}

// 2. Tap top-left
{"tool": "ui_interaction", "arguments": {"action": "tap", "target": {"x": 78, "y": 169}}}
// Screenshot shows: "X: 78 Y: 169"

// 3. Tap top-right  
{"tool": "ui_interaction", "arguments": {"action": "tap", "target": {"x": 312, "y": 169}}}
// Screenshot shows: "X: 312 Y: 169"

// 4. Tap center
{"tool": "ui_interaction", "arguments": {"action": "tap", "target": {"x": 195, "y": 422}}}
// Screenshot shows: "X: 195 Y: 422"

// 5. Tap bottom-left
{"tool": "ui_interaction", "arguments": {"action": "tap", "target": {"x": 78, "y": 675}}}
// Screenshot shows: "X: 78 Y: 675"

// 6. Tap bottom-right
{"tool": "ui_interaction", "arguments": {"action": "tap", "target": {"x": 312, "y": 675}}}
// Screenshot shows: "X: 312 Y: 675"
```

## Interpreting Results

If the displayed coordinates match what you sent, calibration is perfect. If there's a consistent offset:

- **Positive offset**: Actual tap is to the right/below expected
- **Negative offset**: Actual tap is to the left/above expected

Apply the inverse of this offset to future taps for accuracy.

## Notes

- The calibration view remains open after completion to show all results
- You can return to normal testing by launching any other app
- Calibration data persists until the next calibration session
- The test host app continues to run in the background for XCUITest support