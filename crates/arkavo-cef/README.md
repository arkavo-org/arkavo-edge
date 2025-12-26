# arkavo-cef

Chromium Embedded Framework (CEF) integration for Arkavo Edge with native Blink DOM APIs.

## Features

- **Native Blink DOM Manipulation**: Sub-millisecond DOM control directly from Rust without JavaScript.
- **Zero-JavaScript Architecture**: High-performance rendering with V8 disabled for maximum efficiency and security.
- **Async Communication**: Fully non-blocking architecture using Unix domain sockets and FlatBuffers.
- **Real-Time Event Bridge**: Native streaming of DOM events (clicks, inputs, etc.) back to the Rust agent.
- **GPU Acceleration**: Direct integration with the Chromium compositor for hardware-accelerated UI rendering.
- **Performance Telemetry**: Integrated monitoring of FPS, LCP, and layout execution costs.