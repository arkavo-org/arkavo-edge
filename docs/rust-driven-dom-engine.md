# Rust-Driven DOM Engine

**Issue**: [#273](https://github.com/arkavo-org/arkavo-edge/issues/273)
**Status**: Planning Phase
**Target**: macOS .pkg distribution (not brew)

## Core Concept

Transform Chromium Embedded Framework (CEF) from "a browser running JS" into a **Blink-based rendering engine controlled by Rust**. The AI agent becomes the layout + logic compiler, generating DOM diffs, style changes, and event wiring commands — with the renderer acting as a "headless UI GPU".

## Architecture Overview

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

## High-Speed Communication Layer (UDS)

### Transport
- **Medium**: Unix Domain Socket (`/tmp/arkavo_dom.sock`)
- **Latency**: ~3–20 µs
- **Protocol**: Binary message format (FlatBuffers or Cap'n Proto)
- **Concurrency**: Single reader/writer thread per side, lock-free ring buffer for zero-copy message passing

### Lifetime
1. Agent spawns renderer process
2. Renderer connects to UDS immediately and waits for commands

### Message Schema

```rust
table DOMCommand {
  id: uint32;
  op: DOMOp;
  selector: string;
  payload: string;   // innerHTML, CSS text, etc.
}

enum DOMOp: byte {
  ReplaceInnerHTML,
  SetAttribute,
  SetStyle,
  RemoveNode,
  AppendNode,
  DispatchEvent
}

table DOMFeedback {
  id: uint32;
  status: Status;
  execTimeNs: uint64;
  message: string;
}
```

## Renderer-Side: Native DOM Manipulation

### Approach
- Build a small C++/Rust hybrid library inside the CEF render process
- Disable or omit V8 entirely (`--disable-javascript`), or compile CEF with `ENABLE_V8=0`
- Use Blink's internal `Document`, `Element`, `Node`, and `EventDispatcher` APIs directly

### Key Operations
- `Document::getElementById` / `QuerySelector`
- `Element::setInnerHTML`, `setAttribute`, `style()->setProperty`
- `EventDispatcher::DispatchEvent` for click/input/etc.

### Performance (M-series Mac)
- Simple `innerHTML` swap: ~20–80 µs
- Attribute/style change: ~5–15 µs
- Event dispatch: ~10–40 µs

## Feedback Channel (Real-Time Telemetry)

Renderer streams feedback back to Rust immediately:

- ✅ Execution timing (per op)
- ✅ DOM structure diffs (optional)
- ✅ Runtime errors (invalid selector, paint failure)
- ✅ Event telemetry (click, input, `animationend`, etc.)
- ✅ Paint timing (LCP, FPS, layout cost)

### Feedback Loop Example

```
AI agent → [ReplaceInnerHTML] → Renderer
Renderer → [Applied t=84µs] → AI agent
AI agent → [DispatchEvent] → Renderer
Renderer → [Event click captured] → AI agent
```

**Round-trip time**: < 1 ms in most cases

## Event Wiring (Zero-JS)

- All events (click, change, scroll, etc.) are registered at the renderer level
- When triggered, they're serialized back over the UDS to Rust
- Rust decides how to respond — including sending more DOM changes or triggering further UI flows

This lets the AI agent drive interaction like a user without a JS event handler ever existing.

## DOM Diff Pipeline (Optional Optimization)

Instead of sending whole HTML blobs, send structural diffs:
- `InsertNode`
- `RemoveNode`
- `ReplaceText`
- `SetAttribute`

This reduces payloads and mutation cost dramatically — enabling sub-100 µs updates for small diffs.

## GPU & Rendering

- Blink handles CSS layout, compositing, and GPU rendering natively
- The renderer still runs the compositor pipeline (Skia + GPU)
- Can tap into frame timing callbacks for feedback (e.g., frame-ready time, paint cost)

## Performance Targets (M-series Mac)

| Operation                        | Latency         |
| -------------------------------- | --------------- |
| DOM command parse & dispatch     | ~5–20 µs        |
| `InnerHTML` swap                 | ~20–80 µs       |
| Attribute/style update           | ~5–15 µs        |
| Event dispatch → feedback        | ~50–200 µs      |
| Full round-trip (agent ↔ render) | < 1 ms          |

## Key Benefits

- 🚀 **No JS runtime at all** — no parsing, GC, or sandbox overhead
- ⚡ **Sub-millisecond closed loop** — fast enough for AI-driven iterative DOM synthesis
- 🔒 **Reduced attack surface** — no script injection, no `eval`
- 🧠 **Total Rust control** — DOM is now a render target, not an app runtime
- 🧰 **Future-proof** — same architecture scales to WebGPU, Canvas, or even 3D scene graphs

## Migration Path (Practical)

### Phase 1: CEF + Minimal JS Glue (1–2 weeks)
- Integrate CEF into arkavo-agui
- Use minimal JavaScript bridge for DOM manipulation
- Test basic UI generation pipeline
- Package in macOS .pkg only

### Phase 2: Native DOM API Bridge (2–4 weeks)
- Replace JS calls with render-side DOM API bridge (C++/Rust)
- Implement UDS communication layer
- Migrate to binary DOM command protocol
- Performance benchmarking

### Phase 3: Remove V8, Deploy Native Protocol (4–6 weeks)
- Compile CEF with V8 disabled (`ENABLE_V8=0`)
- Full native DOM manipulation via Blink APIs
- Achieve target performance metrics
- Production hardening

## Distribution Notes

### macOS .pkg Only
- CEF binaries will be bundled **only in the macOS .pkg installer**
- **Not included in Homebrew** distribution due to size and complexity
- Brew version remains lightweight, web-based UI only

### Binary Size Considerations
- CEF adds ~100-150 MB to distribution
- Target: Keep .pkg under 200 MB total
- Brew binary maintains <60 MB target

## Integration with Current Codebase

### Files Affected
- `crates/arkavo-agui/src/gateway.rs` - Add CEF process management
- `crates/arkavo-agui/Cargo.toml` - Add CEF dependencies (optional feature)
- New crate: `crates/arkavo-cef/` - CEF wrapper and UDS bridge

### Feature Flags
```toml
[features]
default = ["web-ui"]
web-ui = []  # Current warp-based web UI (shell.html)
cef-ui = ["dep:cef-rust", "dep:flatbuffers"]  # CEF-based native UI
```

### Backward Compatibility
- Web UI remains default for all platforms
- CEF UI opt-in for macOS .pkg builds
- Graceful fallback if CEF not available

## Result

A Rust-native AI rendering loop where the DOM is just a high-performance scene graph. Code generation, testing, error feedback, and UI evolution all happen inside Rust — with a continuous, <1 ms feedback loop.

## See Also

- [Issue #273: Rust-Driven DOM Engine](https://github.com/arkavo-org/arkavo-edge/issues/273)
- [arkavo-agui README](../crates/arkavo-agui/README.md)
- [Archived Dashboard UI](./archived-ui/README.md)
