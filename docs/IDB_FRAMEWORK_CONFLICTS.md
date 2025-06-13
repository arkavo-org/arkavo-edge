# IDB Framework Conflicts Resolution Guide

## Overview

When using the embedded IDB companion in Arkavo Edge, you may encounter framework conflicts between the IDB frameworks and system frameworks. This document explains the issue and provides solutions.

## The Problem

The embedded IDB companion includes several frameworks (FBControlCore.framework, FBDeviceControl.framework, etc.) that contain classes also defined in system frameworks. This leads to errors like:

```
Class FBProcess is implemented in both /System/Library/PrivateFrameworks/FrontBoard.framework/Versions/A/FrontBoard 
and /private/var/folders/.../arkavo_idb/Frameworks/FBControlCore.framework
```

Additionally, IDB companion may fail to bind to its default port (10882) if another instance is already running.

## Solutions

### 1. Use System-Installed IDB (Recommended)

The most reliable solution is to use the system-installed IDB companion from Homebrew:

```bash
# Install IDB via Homebrew
brew install facebook/fb/idb-companion

# Force Arkavo to use system IDB
export ARKAVO_USE_SYSTEM_IDB=1

# Run your tests
cargo test
```

### 2. Automatic Fallback

The IDB wrapper now automatically detects framework conflicts and attempts to fall back to system IDB if available. No action required - just ensure IDB is installed via Homebrew.

### 3. Kill Existing IDB Processes

If you see port binding errors:

```bash
# Kill any existing IDB processes
pkill idb_companion

# Check if port 10882 is in use
lsof -i :10882
```

### 4. Environment Variables

You can control IDB behavior with these environment variables:

- `ARKAVO_USE_SYSTEM_IDB=1` - Force use of system-installed IDB
- `DYLD_DISABLE_LIBRARY_VALIDATION=1` - Allow loading of unsigned frameworks (set automatically)
- `DYLD_FORCE_FLAT_NAMESPACE=1` - Force flat namespace for symbol resolution (set automatically)

## Technical Details

The IDB wrapper implements several strategies to handle framework conflicts:

1. **DYLD Environment Variables**: Sets various DYLD_* variables to control framework loading
2. **System IDB Detection**: Checks common Homebrew locations for system IDB
3. **Automatic Fallback**: Detects framework conflicts and switches to system IDB
4. **Port Management**: Attempts to find available ports if default is in use

## Troubleshooting

### Framework conflicts persist

1. Ensure all IDB processes are killed: `pkill -9 idb_companion`
2. Clear temp directories: `rm -rf /tmp/arkavo_idb`
3. Use system IDB: `export ARKAVO_USE_SYSTEM_IDB=1`

### Port binding issues

1. Check what's using port 10882: `lsof -i :10882`
2. Kill the process: `kill -9 <PID>`
3. Or use a different port (IDB wrapper will find one automatically)

### System IDB not found

Ensure IDB is installed and in your PATH:

```bash
# Install IDB
brew install facebook/fb/idb-companion

# Check installation
which idb_companion

# Should output something like:
# /opt/homebrew/bin/idb_companion (Apple Silicon)
# /usr/local/bin/idb_companion (Intel Mac)
```

## Future Improvements

We're working on:

1. Better framework isolation techniques
2. Automatic port allocation for multiple IDB instances
3. Improved error messages and recovery strategies