# arkavo-device-identity

Stable, device-bound identity management for autonomous Arkavo agents.

## Features

- **Persistent Device ID**: Stable UUIDv4 identifiers that persist across agent restarts and updates.
- **Secure Platform Storage**: Non-interactive, file-based storage in platform-standard agent data locations (macOS, Linux, Windows).
- **NPE (Non-Person Entity) Support**: Designed for zero-configuration, autonomous operation without human interaction.
- **Device Binding**: Cryptographically binds agent identity to the specific hardware instance.
- **Attestation Integration**: Serves as the primary identity for hardware-backed attestation and authorization.