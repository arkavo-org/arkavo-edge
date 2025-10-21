# arkavo-cef

Chromium Embedded Framework (CEF) integration for Arkavo Edge with **native Blink DOM API** manipulation. This crate provides a high-performance, sub-millisecond render loop where Rust controls the DOM directly without JavaScript.

## Core Concept

Instead of using CEF as "a browser running JS", arkavo-cef treats it as a **Blink-based rendering engine controlled by Rust**. The AI agent becomes the layout + logic compiler, generating DOM diffs, style changes, and event wiring commands — with the renderer acting as a "headless UI GPU".

## Architecture

```text
┌───────────────────────────────┐
│      Rust AI Agent            │
│  (code generator & planner)   │
└───────▲───────────────┬───────┘
        │ Binary diff   │ Feedback (error, perf, DOM events)
        │               │
   (Unix domain socket, <0.2ms)
        │               │
┌───────┴───────────────▼───────┐
│     Render Process (CEF)      │
│  - No V8 / JS runtime          │
│  - DOM tree manipulated via    │
│    Blink internal APIs         │
│  - Event dispatch engine       │
│  - GPU compositor              │
└───────────────────────────────┘
```

## Features

- **Zero JavaScript**: V8 disabled at compile time (`ENABLE_V8=0`)
- **Native DOM APIs**: Direct Blink `Document`, `Element`, `Node` manipulation
- **Sub-millisecond latency**: Unix domain socket with FlatBuffers protocol
- **Event-driven**: DOM events serialized back to Rust for AI decision-making
- **Performance telemetry**: FPS, LCP, layout cost monitoring
- **Production-ready**: Process crash recovery, memory leak detection, sanitization

## Performance Targets (M-series Mac)

| Operation | Target Latency |
|-----------|----------------|
| UDS latency | <20 µs |
| DOM command execution | <50 µs |
| Full round-trip (Rust ↔ CEF) | <1 ms |
| UI generation cycle | <50 ms |

## Building

### Automated Setup (Recommended)

The build system automatically downloads and builds CEF when needed:

```bash
# Build with CEF support (downloads CEF automatically)
cargo build -p arkavo-cef

# Or build the entire workspace with CEF
cargo build --features cef-ui
```

On first build, CEF (~100MB) will be downloaded and configured automatically. This takes 5-10 minutes.

### Manual Setup (Advanced)

If you prefer to set up CEF manually:

1. **Run the setup script**:
   ```bash
   ./scripts/setup-cef.sh
   ```

2. **Build the crate**:
   ```bash
   cargo build -p arkavo-cef
   ```

### Prerequisites

- **CMake**: `brew install cmake`
- **macOS**: Currently only macOS is supported
- **Disk space**: ~300MB for CEF distribution and build artifacts

## Usage

```rust
use arkavo_cef::{CefRenderer, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let mut renderer = CefRenderer::new("/path/to/arkavo-cef-renderer").await?;

    renderer.commands()
        .replace_inner_html("#content", "<div>Hello, World!</div>")
        .await?;

    renderer.commands()
        .set_style("#content", "background-color", "blue")
        .await?;

    renderer.commands()
        .add_event_listener("#button", "click")
        .await?;

    renderer.shutdown()?;
    Ok(())
}
```

## Protocol

Communication uses FlatBuffers for zero-copy serialization:

### DOMCommand
```fbs
table DOMCommand {
    id: uint32;
    op: DOMOp;
    selector: string;
    payload: string;
    property: string;
}
```

### DOMFeedback
```fbs
table DOMFeedback {
    id: uint32;
    status: FeedbackStatus;
    exec_time_ns: uint64;
    message: string;
    telemetry: Telemetry;
}
```

## Supported DOM Operations

- `ReplaceInnerHTML`: Replace element's innerHTML
- `SetAttribute`: Set element attribute
- `SetStyle`: Set CSS property
- `SetTextContent`: Set text content
- `RemoveNode`: Remove element from DOM
- `AddEventListener`: Register event listener
- `QuerySelector`: Query DOM for elements

## Distribution

- **macOS .pkg**: Includes CEF binaries (~100-150MB)
- **Homebrew**: Uses web-ui fallback (no CEF bundling)

## Security

- **Selector sanitization**: Prevents injection attacks
- **Rate limiting**: Prevents UI thrashing
- **Process isolation**: CEF runs in separate process
- **No eval**: No JavaScript execution path

## Future Enhancements

- DOM diffing algorithm (minimal command generation)
- WebGPU support for 3D scene graphs
- Canvas-based rendering path
- Multi-window support

## See Also

- [Rust-Driven DOM Engine Design Doc](../../docs/rust-driven-dom-engine.md)
- [Issue #273](https://github.com/arkavo-org/arkavo-edge/issues/273)
