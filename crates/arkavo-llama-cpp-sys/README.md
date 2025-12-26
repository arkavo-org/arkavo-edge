# arkavo-llama-cpp-sys

Low-level FFI bindings and automated build system for llama.cpp.

## Features

- **Automated Compilation**: Build-time compilation of llama.cpp using CMake with support for various hardware backends.
- **FFI Bindings**: bindgen-generated Rust headers for type-safe interaction with the underlying C++ inference engine.
- **Hardware Acceleration**: Configurable build support for Metal (macOS), CUDA (Linux/Windows), and Vulkan.
- **Cross-Platform Compatibility**: Optimized build scripts for unified deployment across macOS, Linux, and Windows.
- **Static Linking**: Seamlessly integrates llama.cpp into Rust binaries as a statically linked dependency.
