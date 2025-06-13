# IDB Auto-Recovery Enhancement

## Overview

This document describes the enhancements made to the IDB (iOS Debug Bridge) auto-recovery system to handle the specific case where IDB companion is running but not properly connected to the simulator.

## Problem

The calibration system was experiencing issues where:
- IDB companion process was running (`companion_running=true`)
- But the port was not accessible (`connected=false`)
- This "stuck" state prevented successful iOS simulator interactions
- The existing recovery logic didn't specifically handle this scenario

## Solution

### 1. Enhanced IDB Recovery Module (`idb_recovery.rs`)

Added a new method `recover_stuck_companion()` that specifically handles the stuck companion scenario:

- **Clears connection tracking**: Removes all devices from the `CONNECTED_DEVICES` set
- **Graceful shutdown attempt**: Sends SIGTERM to IDB companion processes first
- **Force kill if needed**: Uses SIGKILL if graceful shutdown fails
- **Port cleanup**: Kills any processes holding port 10882
- **Temp file cleanup**: Removes IDB lock files and sockets
- **Re-initialization**: Forces IDB wrapper to re-initialize with embedded binary
- **Fallback to system IDB**: If embedded IDB fails, tries system IDB as fallback

### 2. Enhanced Connection Recovery

Updated `force_reconnect_device()` to:
- Clear device from connection tracking
- Attempt explicit disconnect via IDB
- Force reconnect with explicit port if needed
- Retry connection with port 10882 if initial attempt fails

### 3. Calibration Server Integration

The calibration server now:
- Detects when `companion_running=true` but `port_accessible=false`
- Uses the targeted `recover_stuck_companion()` method for this specific case
- Falls back to general recovery for other scenarios
- Implements a watchdog timer that checks for stuck states during calibration
- Automatically attempts recovery if no successful taps for 15 seconds

## Key Code Changes

### IdbRecovery Module
- Added `recover_stuck_companion()` method for targeted recovery
- Enhanced `force_reconnect_device()` with explicit connection attempts
- Added port cleanup using `lsof` to find and kill processes on port 10882

### CalibrationServer
- Updated `check_idb_health()` to use targeted recovery
- Enhanced watchdog to detect and handle stuck companion
- Improved tap failure handling to use appropriate recovery method

## Usage

The auto-recovery is automatic and requires no user intervention. It activates when:

1. **During health checks**: If IDB companion is detected as stuck
2. **During tap failures**: If a tap fails due to IDB issues
3. **During timeouts**: If operations timeout (indicating stuck IDB)
4. **Via watchdog**: If no successful operations for 15 seconds

## Testing

Created `idb_recovery_test.rs` with tests for:
- Stuck companion detection and recovery
- Device reconnection functionality

## Benefits

1. **Improved reliability**: Automatically recovers from stuck IDB states
2. **Reduced manual intervention**: No need to manually kill IDB processes
3. **Better diagnostics**: Clear logging of IDB state and recovery attempts
4. **Graceful degradation**: Falls back to system IDB if embedded IDB has issues
5. **Targeted recovery**: Uses appropriate recovery method based on the specific issue

## Notes

- The recovery system uses the embedded IDB binary from build.rs
- It prefers embedded IDB but can fall back to system IDB if needed
- All recovery operations include appropriate wait times for stabilization
- The system clears connection state to ensure clean reconnection