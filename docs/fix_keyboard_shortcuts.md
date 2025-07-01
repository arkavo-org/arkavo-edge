# Keyboard Shortcut Issues Analysis

## Issues Found:

1. **Tab cycling not working**: The Tab handler is correct, but may not be reached if:
   - The app is in configuration mode
   - The available_models list is empty
   - The input is not focused

2. **Ctrl+E (Helix) should be disabled if not available**: Currently shows in help even when Helix isn't found

3. **Ctrl+D and Ctrl+T not working**: The handlers exist but may have conditions preventing execution

## Root Causes:

1. **Configuration Mode**: When `configuration_mode != ConfigurationMode::None`, ALL key events are consumed (line 705)

2. **Helix Availability**: The help always shows Ctrl+E, but it should only show when Helix is available

3. **Debug View (Ctrl+D)**: Works but only changes `view_mode`, may not be visible in UI

4. **MCP Tools Dialog (Ctrl+T)**: The `show_mcp_tools_dialog` method exists but implementation may be incomplete

## Fixes Needed:

1. Add debug logging to Tab handler to verify it's being reached
2. Only show Ctrl+E in help when Helix is available
3. Ensure Ctrl+D and Ctrl+T work regardless of configuration mode
4. Fix active model display to show current selection