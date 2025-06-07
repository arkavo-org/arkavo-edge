# idb_companion Integration in Arkavo

## Overview

This document describes the integration of Meta's idb_companion into Arkavo for reliable iOS simulator UI automation.

## Architecture

### Embedded Binary Approach

We embed the idb_companion binary directly into the Arkavo executable at build time:

1. **Build Time**: The `build.rs` script embeds the platform-specific idb_companion binary
2. **Runtime**: The binary is extracted to a temporary directory and executed as needed
3. **Single Binary**: Users get a single Arkavo executable with no external dependencies

### Key Components

1. **`build.rs`**: Downloads/embeds the idb_companion binary
2. **`idb_wrapper.rs`**: Runtime extraction and API wrapper
3. **`ios_tools.rs`**: Integration with existing UI automation

## Implementation Status

### ✅ Completed
- Build script infrastructure for embedding binaries
- Runtime extraction and execution wrapper
- API for tap, swipe, type_text, and button press
- Integration with existing ios_tools as primary method

### 🚧 TODO
- Download actual idb_companion binaries from Meta/Facebook releases
- Add checksum verification for downloaded binaries
- Implement proper error handling for all edge cases
- Add integration tests

## Usage

### For Developers

1. Download idb_companion binaries:
   ```bash
   ./scripts/download_idb_companion.sh
   ```

2. Build Arkavo with embedded idb_companion:
   ```bash
   cargo build -p arkavo-test --release
   ```

### For End Users

No action required! The idb_companion is embedded in the Arkavo binary.

## Benefits

1. **Reliable Tapping**: Direct coordinate mapping without window chrome calculations
2. **No External Dependencies**: Everything is embedded
3. **Cross-Architecture**: Supports both Intel and Apple Silicon
4. **Offline Operation**: No runtime downloads needed

## Technical Details

### Coordinate System

idb_companion uses device logical coordinates directly:
- iPhone 15 Pro: 393×852 points
- No window chrome calculations needed
- Works regardless of simulator window position or scale

### Supported Operations

```rust
// Tap at coordinates
IdbWrapper::tap(device_id, x, y).await

// Swipe gesture
IdbWrapper::swipe(device_id, start_x, start_y, end_x, end_y, duration).await

// Type text
IdbWrapper::type_text(device_id, "Hello World").await

// Press hardware button
IdbWrapper::press_button(device_id, "home").await
```

### Error Handling

The implementation includes fallback mechanisms:
1. Try idb_companion (most reliable)
2. Fall back to XCTest bridge (if available)
3. Fall back to AppleScript (least reliable)

## Security Considerations

- Binary checksums should be verified
- Temporary extraction directory is cleaned up
- No network access required at runtime

## Future Improvements

1. Automatic binary updates during build
2. Support for real devices (not just simulators)
3. More sophisticated gesture support
4. Integration with visual element detection