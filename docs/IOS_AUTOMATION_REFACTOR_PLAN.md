# iOS Automation Refactor Plan

## Goal
Replace the fragile AppleScript-based coordinate tapping with the more reliable XCTest bridge approach.

## Current State Analysis

### Problems with Current Implementation
1. **Hardcoded Bezels in `ios_tools.rs`** (lines 698-702):
   ```rust
   set titleBarHeight to 28
   set sideBezel to 70
   set topBezel to 85
   set bottomBezel to 120
   ```
   These values are guesses that break when:
   - Simulator window is scaled
   - Different device types are used
   - Simulator chrome changes in new Xcode versions

2. **Fallback Logic is Fragile**: The code tries to find device screen via Accessibility API but falls back to hardcoded estimates when it fails.

3. **No Reliable Device Screen Detection**: The AppleScript attempts to find an AXGroup element representing the device screen, but this is unreliable.

## Proposed Solution

### Phase 1: Improve XCTest Integration
1. **Auto-initialize XCTest**: Modify `ui_interaction` tool to automatically run XCTest setup if not already initialized
2. **Better Connection Management**: Improve XCTest bridge connection handling and recovery
3. **Remove AppleScript Fallback**: Stop using AppleScript for coordinate-based taps

### Phase 2: Enhanced Error Handling
1. **Clear Setup Instructions**: When XCTest isn't available, provide step-by-step setup guide
2. **Connection Status Tool**: Add a tool to check XCTest connection status
3. **Automatic Retry**: Implement retry logic for XCTest connection

### Phase 3: Remove Legacy Code
1. **Deprecate AppleScript Tap**: Mark the AppleScript implementation as deprecated
2. **Update Documentation**: Update all docs to recommend XCTest approach
3. **Clean Up Code**: Remove the complex bezel calculation logic

## Implementation Steps

### Step 1: Enhance XCTest Auto-Setup
```rust
// In ui_interaction execute method
if !xctest_available && (has_text_target || has_accessibility_target) {
    // Try to auto-setup XCTest
    match self.setup_xctest_runner().await {
        Ok(_) => {
            // Continue with XCTest approach
        }
        Err(e) => {
            // Return helpful error with setup instructions
        }
    }
}
```

### Step 2: Create XCTest Status Tool
```rust
pub struct XCTestStatusKit {
    schema: ToolSchema,
}

impl XCTestStatusKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "xctest_status".to_string(),
                description: "Check XCUITest setup and connection status".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        }
    }
}
```

### Step 3: Simplify Coordinate Tapping
```rust
// Remove all AppleScript code and use only XCTest
if use_xctest && xctest_command.is_some() {
    // Use XCTest for ALL tapping (coordinates, text, accessibility)
} else {
    // Return error - no fallback to AppleScript
    return Ok(serde_json::json!({
        "error": {
            "code": "XCTEST_REQUIRED",
            "message": "XCUITest must be initialized for UI interactions",
            "solution": "Run setup_xcuitest tool first"
        }
    }));
}
```

## Benefits

1. **Reliability**: XCTest works with device coordinates, no window position guessing
2. **Simplicity**: Remove ~200 lines of fragile AppleScript code
3. **Consistency**: One method for all UI interactions
4. **Maintainability**: No hardcoded bezel values to update

## Migration Guide

### For Existing Code
1. Replace coordinate calculations that account for bezels
2. Use device logical coordinates directly (e.g., iPhone 15 Pro: 393x852)
3. Always initialize XCTest before UI automation

### For New Features
1. Always use XCTest bridge for UI interactions
2. Prefer text/accessibility ID over coordinates
3. Use `analyze_layout` + AI vision for finding elements

## Testing Plan

1. **Unit Tests**: Test XCTest bridge initialization and commands
2. **Integration Tests**: Verify tapping works across different devices
3. **Edge Cases**: Test with scaled simulator windows, different orientations

## Timeline

- Phase 1: 2-3 days (Improve XCTest integration)
- Phase 2: 1-2 days (Enhanced error handling)
- Phase 3: 1 day (Remove legacy code)

## Risks and Mitigations

1. **Risk**: Breaking existing tests that use AppleScript
   - **Mitigation**: Add compatibility mode during transition

2. **Risk**: XCTest compilation failures
   - **Mitigation**: Pre-compile and cache XCTest bundle

3. **Risk**: Performance impact of auto-setup
   - **Mitigation**: Cache setup state, only initialize once

## Success Criteria

1. All UI interactions work through XCTest
2. No hardcoded bezel values in codebase
3. Clear error messages when setup needed
4. Tests pass regardless of simulator window position/scale