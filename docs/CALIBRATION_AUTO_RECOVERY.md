# Calibration Auto-Recovery System

This document describes the automatic recovery mechanisms added to the calibration manager to handle stuck IDB operations.

## Overview

The calibration manager now includes automatic detection and recovery from stuck IDB operations, eliminating the need for manual intervention when IDB companion hangs during the calibration process.

## Key Features

### 1. Watchdog Timer

- **Timeout**: 15 seconds without successful taps triggers auto-recovery
- **Detection**: Monitors time since last successful tap
- **One-time recovery**: Prevents recovery loops by tracking attempts

### 2. Per-Tap Timeout

- **Timeout**: 10 seconds per tap operation
- **Async handling**: Non-blocking tap execution with timeout
- **Immediate recovery**: Triggers recovery on timeout

### 3. Auto-Recovery Process

When stuck operations are detected:

1. **IDB Process Termination**: Kills all idb_companion processes
2. **Cache Cleanup**: Removes IDB cache directories
3. **Connection Reset**: Clears tracked device connections
4. **Operation Retry**: Automatically retries failed tap

### 4. Enhanced Status Reporting

Status responses now include:

```json
{
  "stuck_warning": "No taps detected in 12 seconds. Auto-recovery will trigger at 15 seconds.",
  "auto_recovery_status": {
    "will_trigger_in": 3,
    "description": "Automatic IDB recovery will attempt to fix the issue"
  },
  "phase": {
    "name": "Auto-Recovery",
    "status": "recovering",
    "recovery_steps": [
      "Terminating stuck IDB companion processes",
      "Clearing IDB cache",
      "Re-initializing IDB connection",
      "Retrying tap sequence"
    ]
  }
}
```

## Implementation Details

### Watchdog Timer Logic

```rust
// Track time since last successful tap
let mut last_successful_tap = Instant::now();

// Check if we're stuck (no taps for 15 seconds)
if last_successful_tap.elapsed() > Duration::from_secs(15) && !stuck_recovery_attempted {
    // Trigger auto-recovery
    self.idb_recovery.attempt_recovery().await;
    stuck_recovery_attempted = true;
}
```

### Tap Timeout Handling

```rust
// Wrap tap with 10-second timeout
let tap_result = timeout(
    Duration::from_secs(10),
    spawn_blocking(|| agent.execute_tap(x, y))
).await;

match tap_result {
    Err(_) => {
        // Timeout - trigger recovery
        self.idb_recovery.attempt_recovery().await;
    }
    // ... handle other cases
}
```

## Status Messages

### During Normal Operation

```
"message": "Calibration Phase 2-3: Tap sequence and verification (15s elapsed, 3 taps completed)"
"progress": "3/5 taps completed"
```

### When Stuck Detected

```
"stuck_warning": "No taps detected in 12 seconds. Auto-recovery will trigger at 15 seconds."
"auto_recovery_status": {
  "will_trigger_in": 3
}
```

### During Recovery

```
"message": "Calibration Phase 2: Auto-recovery in progress"
"phase": {
  "name": "Auto-Recovery",
  "status": "recovering"
}
```

## Troubleshooting Information

The calibration tool now provides enhanced troubleshooting info:

```json
{
  "troubleshooting": {
    "stuck_in_initializing": "App may not be detecting taps. Auto-recovery will be attempted after 15 seconds",
    "idb_stuck": "If no taps are detected for 15 seconds, automatic IDB recovery will be triggered"
  },
  "auto_recovery": {
    "enabled": true,
    "watchdog_timeout": "15 seconds",
    "tap_timeout": "10 seconds per tap",
    "description": "Calibration manager will automatically recover from stuck IDB operations"
  }
}
```

## Benefits

1. **No Manual Intervention**: Automatically recovers from stuck states
2. **Faster Resolution**: Detects and fixes issues within 15 seconds
3. **Clear Status Updates**: Real-time information about recovery process
4. **Prevents Timeouts**: Recovers before 60-second calibration timeout
5. **Retry Logic**: Automatically retries failed operations after recovery

## Testing Agent Guidance

For testing agents using the calibration system:

1. **Monitor Status**: Check `auto_recovery_status` field for recovery countdown
2. **Wait for Recovery**: If stuck warning appears, wait 15 seconds for auto-recovery
3. **Check Phase**: Look for "Auto-Recovery" phase to know recovery is in progress
4. **No Manual Action**: The system will handle recovery automatically

## Future Improvements

- Configurable timeout values
- Multiple recovery strategies
- Predictive stuck detection
- Recovery success metrics