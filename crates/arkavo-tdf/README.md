# arkavo-tdf

Core TDF (Trusted Data Format) abstraction and security layer for Arkavo.

## Features

- **TDF Abstraction Layer**: Streaming-first traits and types for GB-scale Trusted Data Format operations.
- **ZTDF-JSON Support**: Native handling of the ZTDF-JSON format for inline payloads in JSON-RPC.
- **KAS Client Integration**: Built-in client for Key Access Service (KAS) with Arkavo OAuth integration.
- **Decoupled Security**: Flexible architecture allowing composition of distinct encryption and transport backends.
- **Streaming Security**: Designed to process large data volumes with cryptographic integrity without OOM risk.
- **OpenTDF Integration**: Optional native support for real TDF encryption via the opentdf-rs ecosystem.
