# arkavo-cef Quick Start Guide

Get started with the Rust-Driven DOM Engine in minutes.

## Prerequisites

### Install CMake

```bash
# macOS
brew install cmake

# Verify installation
cmake --version  # Should be >= 3.19
```

**Note**: CEF will be downloaded automatically on first build. No manual download needed!

## Building

### Automated Build (Recommended)

Simply build the crate - CEF will be set up automatically:

```bash
# Build arkavo-cef (downloads and builds CEF automatically on first run)
cargo build -p arkavo-cef

# Or build with CEF UI support
cargo build --features cef-ui
```

The first build will:
1. Download CEF (~100MB) from Spotify CDN
2. Extract and configure CEF
3. Build the CEF DLL wrapper
4. Build the arkavo-cef Rust bridge

**This takes 5-10 minutes on first build, then is cached.**

### Manual Setup (Optional)

If you prefer manual control:

```bash
# Download and setup CEF
./scripts/setup-cef.sh

# Then build normally
cargo build -p arkavo-cef
```

## Testing

### Unit Tests

```bash
# Test protocol serialization
cargo test -p arkavo-cef

# Test UDS transport
cargo test -p arkavo-cef uds
```

### Integration Test (Manual)

```rust
// examples/test_cef.rs
use arkavo_cef::{CefRenderer, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let renderer_path = std::env::var("ARKAVO_CEF_RENDERER_PATH")
        .unwrap_or_else(|_| {
            "crates/arkavo-cef/cef-bridge/build/arkavo-cef-renderer".to_string()
        });

    println!("Initializing CEF renderer...");
    let mut renderer = CefRenderer::new(&renderer_path).await?;

    println!("Rendering HTML...");
    renderer.commands()
        .replace_inner_html("body", "<h1>Hello from Rust!</h1>")
        .await?;

    println!("Setting style...");
    renderer.commands()
        .set_style("h1", "color", "blue")
        .await?;

    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    println!("Shutting down...");
    renderer.shutdown()?;

    Ok(())
}
```

Run it:
```bash
cargo run --example test_cef --features cef-ui
```

## Usage in arkavo-agui

```rust
use arkavo_agui::{create_renderer, RendererType};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create CEF renderer
    let mut renderer = create_renderer(RendererType::Cef).await?;

    // Generate UI (from arkavo-ui-generator)
    let html = "<div id='app'>Generated UI</div>";
    let css = "#app { padding: 20px; }";

    renderer.render(html, css, "").await?;

    // Incremental update
    renderer.update_element("#app", "<p>Updated content</p>").await?;

    Ok(())
}
```

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `ARKAVO_CEF_RENDERER_PATH` | Path to CEF renderer binary | Auto-detect |
| `ARKAVO_DEBUG` | Enable debug logging | `false` |
| `CEF_ROOT` | CEF distribution path (CMake) | `../../../../vendor/cef` |

## Troubleshooting

### Error: "CEF renderer binary not found"

Set the path explicitly:
```bash
export ARKAVO_CEF_RENDERER_PATH=/path/to/arkavo-cef-renderer
cargo run --example test_cef
```

### Error: "Failed to connect to UDS"

Check socket permissions:
```bash
ls -la /tmp/arkavo_dom_*.sock
rm /tmp/arkavo_dom_*.sock  # Clean up stale sockets
```

### Error: "flatc: command not found"

Install FlatBuffers:
```bash
brew install flatbuffers
```

### Build Errors in C++

Ensure CEF_ROOT is correct:
```bash
cd crates/arkavo-cef/cef-bridge/build
cmake -DCEF_ROOT=$(pwd)/../../../../vendor/cef ..
```

## Performance Benchmarking

```bash
# Run benchmarks (once implemented)
cargo bench -p arkavo-cef

# Measure round-trip latency
RUST_LOG=debug cargo run --example benchmark_latency
```

Expected performance on M-series Mac:
- UDS latency: <20µs
- DOM command: <50µs
- Round-trip: <1ms

## Next Steps

1. Implement event bridge (see `cef-bridge/event_bridge.cc`)
2. Add telemetry collection (see `protocol.fbs::Telemetry`)
3. Run integration tests with arkavo-ui-generator
4. Profile memory usage with Instruments

## Resources

- [Rust-Driven DOM Engine Design](../../docs/rust-driven-dom-engine.md)
- [Implementation Summary](../../docs/rust-driven-dom-implementation-summary.md)
- [CEF Documentation](https://bitbucket.org/chromiumembedded/cef/wiki/Home)
- [FlatBuffers Rust Guide](https://flatbuffers.dev/flatbuffers_guide_use_rust.html)

## Support

For issues or questions:
- GitHub: https://github.com/arkavo-org/arkavo-edge/issues
- Discussions: https://github.com/arkavo-org/arkavo-edge/discussions
