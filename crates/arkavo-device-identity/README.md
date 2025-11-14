# arkavo-device-identity

Device-bound identity management for arkavo-edge agents.

## Purpose

Provides stable, persistent device identifiers for arkavo-edge agents. Each agent is bound to its device and never migrates. The device ID serves as the primary identity for attestation and authorization.

## Features

- **Stable device ID**: Generated once on first run, persists across restarts
- **Platform-secure storage**:
  - macOS: Keychain
  - Linux: File-based storage in `~/.local/share/arkavo/` or optional keyring
  - Windows: File-based storage (Credential Manager support planned)
- **UUID-based**: 128-bit UUIDv4 for global uniqueness

## Usage

```rust
use arkavo_device_identity::{get_or_create_device_id, DeviceId};

// Get existing or create new device ID
let device_id = get_or_create_device_id()?;
println!("Device ID: {}", device_id);

// Access as bytes (for NTDF claims)
let bytes: &[u8; 16] = device_id.as_bytes();
```

## Platform Support

- ✅ macOS (ARM64/x86_64) - File-based (NPE agent mode)
- ✅ Linux (x86_64/ARM64) - File-based or keyring
- ✅ Windows (x86_64) - File-based
- ✅ Raspberry Pi 5 (ARM64 Linux)
- ✅ Arduino UNO R4 WiFi - Planned (embedded storage)

## NPE (Non-Person Entity) Agent Mode

**Zero Configuration Required** - arkavo-edge is designed for device-bound NPE agents that run autonomously without human interaction.

### Storage Locations

Device IDs are stored in platform-specific agent data locations:

- **macOS**: `~/Library/Application Support/arkavo/device_id` (file-based, no user prompts)
- **Linux**: `~/.local/share/arkavo/device_id` (hex-encoded, file permissions 0600)
- **Windows**: `%LOCALAPPDATA%\arkavo\device_id` (planned)

### Why Not Keychain on macOS?

Keychain requires user interaction (password prompts) which breaks autonomous agent operation. File-based storage in `Application Support` is:
- ✅ Standard macOS location for agent/daemon data
- ✅ Non-interactive (no user prompts)
- ✅ Protected by filesystem permissions
- ✅ Survives app updates
- ✅ Works with launchd agents

Keychain code remains available for future interactive use cases (optional feature flag).
