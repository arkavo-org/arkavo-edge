# IDB Companion Monitoring and Recovery

This document describes the IDB companion monitoring and recovery features added to the calibration system.

## Overview

The calibration manager now includes comprehensive IDB companion health monitoring and automatic recovery mechanisms to handle stuck or failed IDB operations during the calibration process.

## Key Features

### 1. IDB Status Monitoring

The calibration status now includes detailed IDB information:

```json
{
  "idb_status": {
    "connected": true,
    "companion_running": true,
    "last_health_check": "2025-06-10T21:30:45Z",
    "last_error": null
  },
  "tap_count": 5,
  "last_tap_time": "2025-06-10T21:30:43Z"
}
```

### 2. Health Checks During Calibration

- **Pre-calibration check**: Verifies IDB is healthy before starting
- **Periodic checks**: During tap sequences to detect issues early
- **Post-failure checks**: After any tap failure to diagnose IDB issues

### 3. Automatic Recovery

When IDB failures are detected, the system automatically:

1. Kills stuck idb_companion processes
2. Clears IDB cache directories
3. Resets connection tracking
4. Retries the failed operation

### 4. IDB Management Tool

A new MCP tool `idb_management` provides manual control:

```json
{
  "tool_name": "idb_management",
  "params": {
    "action": "health_check",
    "device_id": "optional-device-id"
  }
}
```

Available actions:
- `health_check`: Check IDB companion health
- `recover`: Manually trigger recovery process
- `status`: Get detailed IDB status
- `list_targets`: List available devices/simulators

## Calibration Status Enhancements

The calibration status response now includes:

- **tap_count**: Number of successful taps performed
- **last_tap_time**: Timestamp of the last successful tap
- **idb_status**: Detailed IDB health information
- **idb_warning**: Warning if IDB issues are detected
- **stuck_warning**: Warning if no taps detected for >10 seconds

## Usage Examples

### Check Calibration Status with IDB Info

```json
{
  "tool_name": "calibration_manager",
  "params": {
    "action": "get_status",
    "session_id": "cal_DEVICE_ID_TIMESTAMP"
  }
}
```

Response includes:
```json
{
  "status": "validating",
  "elapsed_seconds": 15,
  "tap_count": 3,
  "idb_status": {
    "connected": true,
    "companion_running": true
  }
}
```

### Manually Check IDB Health

```json
{
  "tool_name": "idb_management",
  "params": {
    "action": "health_check"
  }
}
```

### Recover Stuck IDB

```json
{
  "tool_name": "idb_management",
  "params": {
    "action": "recover"
  }
}
```

## Troubleshooting

### Common Issues

1. **Calibration stuck at "initializing"**
   - IDB may not be connected to the device
   - Run `idb_management` health check

2. **No taps detected warning**
   - IDB companion may be stuck
   - Automatic recovery will be attempted
   - Check `idb_status.last_error` for details

3. **Repeated tap failures**
   - IDB companion process may need manual recovery
   - Use `idb_management` recover action

### Recovery Process

The recovery process includes:

1. **Process termination**: Kills all idb_companion processes
2. **Cache cleanup**: Removes IDB cache files
3. **State reset**: Clears connection tracking
4. **Retry mechanism**: Automatically retries failed operations

## Implementation Details

### IdbRecovery Module

Handles recovery operations:
- Process management
- Cache cleanup
- Device responsiveness checks

### CalibrationServer Updates

- Integrated IDB health monitoring
- Automatic recovery on tap failures
- Enhanced status reporting

### CalibrationStatusReport Structure

```rust
pub struct CalibrationStatusReport {
    pub session_id: String,
    pub device_id: String,
    pub start_time: DateTime<Utc>,
    pub elapsed_seconds: u64,
    pub status: String,
    pub idb_status: IdbStatus,
    pub last_tap_time: Option<DateTime<Utc>>,
    pub tap_count: u32,
}

pub struct IdbStatus {
    pub connected: bool,
    pub last_health_check: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub companion_running: bool,
}
```

## Benefits

1. **Improved reliability**: Automatic recovery from IDB failures
2. **Better diagnostics**: Detailed status information
3. **Faster debugging**: Clear indicators of IDB issues
4. **Reduced manual intervention**: Automatic recovery attempts

## Future Improvements

- Configurable recovery timeouts
- Historical IDB health metrics
- Predictive failure detection
- Alternative tap methods when IDB fails