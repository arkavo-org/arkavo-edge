# IDB Framework Dependency Handling

This document explains how the MCP server handles IDB (iOS Development Bridge) framework dependencies and installation.

## Problem

The embedded `idb_companion` binary requires framework dependencies (like `FBControlCore.framework`) that aren't included in the embedded binary. This causes the error:

```
Library not loaded: @rpath/FBControlCore.framework/Versions/A/FBControlCore
```

## Solution

The MCP server now handles this automatically:

1. **Automatic Detection**: Detects missing framework errors
2. **Fallback to System IDB**: Attempts to use system-installed IDB
3. **Guided Installation**: Provides installation instructions via MCP tools

## MCP Tools

### 1. Check IDB Status

```json
{
  "tool_name": "idb_management",
  "params": {
    "action": "health_check"
  }
}
```

This will show:
- If IDB companion is running
- If frameworks are available
- Overall health status

### 2. Install IDB (Recommended)

```json
{
  "tool_name": "idb_management",
  "params": {
    "action": "install"
  }
}
```

This will:
- Check if brew is available
- Install IDB via brew with all dependencies
- Verify installation

### 3. Calibration Status

When calibration detects framework issues, it provides specific guidance:

```json
{
  "idb_warning": "IDB companion has missing framework dependencies. Use 'idb_management' tool with 'install' action to fix.",
  "recommended_action": {
    "tool": "idb_management",
    "action": "install",
    "description": "Install IDB with proper framework dependencies"
  }
}
```

## How It Works

### 1. Embedded IDB Attempt
- First tries the embedded `idb_companion` binary
- Detects "Library not loaded" errors

### 2. System IDB Fallback
- Checks standard locations:
  - `/usr/local/bin/idb_companion` (Intel Macs)
  - `/opt/homebrew/bin/idb_companion` (Apple Silicon)
- Switches to system IDB if found

### 3. Installation Guide
- If no system IDB, provides brew installation instructions
- Can attempt auto-installation if brew is available

## Installation Process

The MCP server installs IDB using the official Facebook/Meta tap:

```bash
brew tap facebook/fb
brew install facebook/fb/idb-companion
```

This ensures:
- All framework dependencies are included
- Proper code signing
- Compatible with the iOS version

## Testing Agent Usage

When the testing agent encounters IDB framework issues:

1. **Don't install IDB manually** - Use the MCP tools
2. **Check the error** - Look for "Library not loaded" in `idb_status.last_error`
3. **Use recommended action** - Follow the `recommended_action` in the response
4. **Install via MCP**:
   ```json
   {
     "tool_name": "idb_management",
     "params": {"action": "install"}
   }
   ```

## Benefits

1. **No Manual Setup**: MCP server handles IDB installation
2. **Automatic Detection**: Framework issues detected automatically
3. **Guided Resolution**: Clear instructions provided in responses
4. **Fallback Support**: Uses system IDB when available
5. **Clean Installation**: Official brew package with all dependencies

## Error Messages

### Before (Confusing)
```
idb_companion tap failed: dyld[26799]: Library not loaded: @rpath/FBControlCore.framework...
```

### After (Clear)
```
IDB companion has missing framework dependencies. Use 'idb_management' tool with 'install' action to fix.
```

## Implementation Details

### IdbInstaller Module
- Detects brew availability
- Checks standard IDB locations
- Handles installation via brew
- Provides fallback instructions

### IdbWrapper Updates
- Detects framework loading errors
- Falls back to system IDB
- Marks preference for future calls
- Provides clear error messages

### Calibration Integration
- Detects IDB issues in status
- Provides recommended actions
- Continues with auto-recovery
- Clear phase information

## Summary

The MCP server now fully handles IDB installation and framework dependencies. Testing agents should:

1. Never install IDB manually
2. Use the `idb_management` tool for installation
3. Follow `recommended_action` guidance
4. Let the MCP server handle the complexity

This ensures consistent, reliable IDB setup across all environments.