# IDB Connection Issue Analysis

## Problem
The calibration is failing with "Device found but IDB not fully connected" error.

## Root Cause
1. The IDB companion binary is successfully extracted to `/tmp/arkavo_idb/idb_companion`
2. However, it's missing required frameworks (FBControlCore.framework, etc.)
3. No system IDB is installed via Homebrew
4. The `list_apps` command fails because IDB can't start without its frameworks
5. This causes the connection verification to fail, even though the device is visible

## Error Flow
1. `check_idb_health` in calibration/server.rs calls `IdbWrapper::list_apps(device_id)` to verify connection
2. `list_apps` tries to run `idb_companion list-apps --udid <device> --only simulator --json`
3. The command fails with dyld error about missing frameworks
4. This is interpreted as "Device found but IDB not fully connected"

## Solution
The user needs to install IDB via Homebrew:
```bash
brew install facebook/fb/idb-companion
```

Then set the environment variable to use system IDB:
```bash
export ARKAVO_USE_SYSTEM_IDB=1
```

## Code Fix Needed
The error message should be more helpful and suggest installing IDB when frameworks are missing.