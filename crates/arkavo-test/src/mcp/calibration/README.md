# MCP UI Automation Calibration System

## Overview

The calibration system automatically tunes UI automation agents to work accurately across different iOS devices and simulators. It uses a reference application with known UI patterns to establish coordinate mappings and interaction adjustments.

## Architecture

### Components

1. **Calibration Agent** - Discovers UI elements and executes interactions
2. **Reference App Interface** - Connects to known test applications
3. **Validation Engine** - Compares results against expected outcomes
4. **Data Store** - Persists calibration profiles per device
5. **Orchestration Server** - Manages calibration sessions and monitoring

### Data Flow

```
Reference App → Agent Discovery → Interaction Testing → Validation → Storage
                     ↓                    ↓                ↓
                UI Elements         Test Results    Calibration Profile
```

## Usage

### Starting Calibration

```rust
use arkavo_test::mcp::calibration::CalibrationServer;

let server = CalibrationServer::new(storage_path)?;
let session_id = server.start_calibration(device_id, None).await?;
```

### Checking Status

```rust
let status = server.get_calibration_status(&session_id).await;
println!("Status: {}", status.status);
```

### Using Calibration Data

```rust
let config = data_store.get_calibration(&device_id);
let adjustment = data_store.get_adjustment_for_element(&device_id, "checkbox");
```

## MCP Tools

The calibration system provides these MCP tools:

- `calibration_manager` - Main calibration operations
- `calibration_status` - Quick status checks

### Example MCP Commands

```bash
# Start calibration
arkavo mcp call calibration_manager --action start_calibration --device_id "ABC123"

# Check status
arkavo mcp call calibration_manager --action get_status --session_id "cal_ABC123_1234567890"

# List calibrated devices
arkavo mcp call calibration_manager --action list_devices

# Enable auto-monitoring
arkavo mcp call calibration_manager --action enable_monitoring --enabled true
```

## Calibration Process

1. **Initialization** - Launch reference app in clean simulator
2. **Discovery** - Scan for UI elements and device parameters
3. **Interaction** - Execute scripted actions (taps, scrolls, etc.)
4. **Validation** - Compare results to expected outcomes
5. **Data Update** - Store coordinate mappings and adjustments
6. **Reporting** - Mark device as calibrated and ready

## Continuous Monitoring

The system automatically monitors calibrations and triggers recalibration when:
- Calibration age exceeds threshold (default: 1 week)
- UI structure changes detected
- Accuracy drops below threshold (95%)

## Best Practices

1. Run calibration after iOS updates
2. Use consistent reference app versions
3. Calibrate all target device types
4. Export calibrations for CI/CD environments
5. Monitor calibration age and validity

## Troubleshooting

### Common Issues

- **"Device not found"** - Ensure simulator is running
- **"Calibration failed"** - Check reference app is installed
- **"Low accuracy"** - May need device-specific adjustments

### Debug Mode

Enable detailed logging:
```rust
env::set_var("RUST_LOG", "arkavo_test::mcp::calibration=debug");
```