# CEF Build System Integration

This document explains how CEF (Chromium Embedded Framework) is integrated into the Arkavo Edge build system.

## Overview

CEF integration follows the same pattern as llama.cpp:
- **Automated download**: CEF is downloaded automatically on first build
- **Cached builds**: Once built, CEF artifacts are reused
- **Git-ignored**: `vendor/cef/` is not committed to the repository
- **Optional setup script**: `scripts/setup-cef.sh` can be run manually

## Architecture

```
arkavo-edge/
├── vendor/
│   ├── llama.cpp/        # Git submodule (for reference)
│   └── cef/              # Downloaded automatically (git-ignored)
│       ├── include/
│       ├── Release/
│       └── build_wrapper/ # CEF DLL wrapper build
├── scripts/
│   └── setup-cef.sh      # CEF download and setup
└── crates/
    └── arkavo-cef/
        ├── build.rs      # Integrates CEF into cargo build
        └── cef-bridge/   # Our C++ bridge
            ├── CMakeLists.txt
            └── *.{cc,h}
```

## Build Flow

### First Build (Cold Start)

1. **User runs**: `cargo build -p arkavo-cef`

2. **build.rs detects**: CEF not found at `vendor/cef/`

3. **build.rs runs**: `scripts/setup-cef.sh`
   - Downloads CEF from Spotify CDN (~100MB)
   - Extracts to `vendor/cef/`
   - Builds CEF DLL wrapper
   - Takes ~5-10 minutes

4. **build.rs compiles**: `cef-bridge/` C++ code with CMake
   - Links against CEF framework
   - Produces `arkavo-cef-renderer` binary

5. **cargo compiles**: Rust code in `arkavo-cef`

### Subsequent Builds (Warm Start)

1. **build.rs detects**: CEF already exists

2. **build.rs checks**: CEF DLL wrapper already built

3. **build.rs compiles**: Only rebuilds if C++ bridge changed

4. **cargo compiles**: Rust code

**Build time**: <30 seconds (only Rust + changed C++ files)

## Setup Script

`scripts/setup-cef.sh` handles CEF setup:

```bash
#!/bin/bash
# 1. Downloads CEF from https://cef-builds.spotifycdn.com
# 2. Extracts to vendor/cef/
# 3. Builds CEF DLL wrapper with CMake
# 4. Cleans up archive
```

**Features**:
- Idempotent: Safe to run multiple times
- Verifies existing installation
- Uses system curl or wget
- Platform-aware (macOS only for now)

## build.rs Integration

`crates/arkavo-cef/build.rs` integrates into Cargo build:

```rust
fn main() {
    // 1. Check platform (macOS only initially)
    if !target.contains("darwin") {
        println!("cargo:warning=CEF only supported on macOS");
        return;
    }

    // 2. Check if CEF exists
    if !cef_root.exists() {
        eprintln!("Run: ./scripts/setup-cef.sh");
        return; // Don't fail - allow build without CEF
    }

    // 3. Check if DLL wrapper built
    if !wrapper_lib.exists() {
        Command::new("bash")
            .arg("../../scripts/setup-cef.sh")
            .status()?;
    }

    // 4. Build our C++ bridge with CMake
    let dst = cmake::Config::new("cef-bridge")
        .define("CEF_ROOT", cef_root)
        .build();

    // 5. Link CEF framework and system libraries
    println!("cargo:rustc-link-lib=framework=CEF");
    println!("cargo:rustc-link-lib=c++");
}
```

**Key decisions**:
- **Non-fatal**: Missing CEF warns but doesn't fail build
- **Auto-repair**: Automatically runs setup script if needed
- **Cargo integration**: Uses `cmake` crate for C++ compilation

## CMakeLists.txt

`crates/arkavo-cef/cef-bridge/CMakeLists.txt`:

```cmake
cmake_minimum_required(VERSION 3.19)
project(arkavo-cef-bridge)

set(CEF_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../../vendor/cef")
find_package(CEF REQUIRED)

add_executable(arkavo-cef-renderer
    main.cc
    cef_app.cc
    dom_executor.cc
    uds_client.cc
)

target_link_libraries(arkavo-cef-renderer
    libcef_lib
    libcef_dll_wrapper
    ${CEF_STANDARD_LIBS}
)
```

## File Sizes

| Component | Size | Cached? |
|-----------|------|---------|
| CEF download | ~100 MB | No (deleted) |
| CEF extracted | ~180 MB | Yes (vendor/cef/) |
| DLL wrapper build | ~50 MB | Yes (build_wrapper/) |
| Our bridge build | ~5 MB | Yes (cef-bridge/build/) |
| **Total** | **~235 MB** | - |

## Platform Support

### Current: macOS

- CEF version: 131.2.7 (Chromium 131.0.6778.86)
- Architecture: x86_64 + arm64 (Universal Binary)
- Frameworks: Metal, AppKit, Foundation

### Future: Linux

Planned support with similar download mechanism:
```bash
# Linux CEF download
CEF_PLATFORM="linux64"
```

### Future: Windows

Planned support with MSVC toolchain:
```bash
# Windows CEF download
CEF_PLATFORM="windows64"
```

## Troubleshooting

### "CEF not found"

**Symptom**: Warning during build
```
cargo:warning=CEF not found - skipping CEF bridge build
```

**Solution**:
```bash
./scripts/setup-cef.sh
cargo build -p arkavo-cef
```

### "CEF DLL wrapper not built"

**Symptom**: Build error about missing `libcef_dll_wrapper.a`

**Solution**:
```bash
# Manual rebuild
cd vendor/cef
rm -rf build_wrapper
cd ../../
./scripts/setup-cef.sh
```

### "CMake Error: Could not find CEF"

**Symptom**: CMake can't locate CEF_ROOT

**Solution**:
```bash
# Verify CEF location
ls -la vendor/cef/include/cef_version.h

# If missing, re-download
rm -rf vendor/cef
./scripts/setup-cef.sh
```

### Slow initial build

**Expected**: First build takes 5-10 minutes
- Downloading CEF: ~2 minutes
- Building DLL wrapper: ~3 minutes
- Building our bridge: ~1 minute
- Compiling Rust: ~1 minute

**Subsequent builds**: <30 seconds

## Comparison with llama.cpp

| Feature | llama.cpp | CEF |
|---------|-----------|-----|
| Location | `vendor/llama.cpp` | `vendor/cef` |
| Setup | Git submodule | Download script |
| Size | ~500 MB (with models) | ~235 MB |
| Build time | ~5 minutes | ~5-10 minutes |
| Platforms | All | macOS (initially) |
| Git-tracked | Yes (submodule) | No (git-ignored) |

## CI/CD Integration

For GitHub Actions:

```yaml
- name: Setup CEF
  run: |
    ./scripts/setup-cef.sh

- name: Build with CEF
  run: |
    cargo build --features cef-ui
```

Cache CEF between runs:

```yaml
- name: Cache CEF
  uses: actions/cache@v3
  with:
    path: vendor/cef
    key: cef-${{ runner.os }}-131.2.7
```

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `CEF_ROOT` | Override CEF location | `../../vendor/cef` |
| `ARKAVO_CEF_RENDERER_PATH` | Override renderer binary | Auto-detect |

## Development Workflow

### Clean rebuild

```bash
# Remove all CEF artifacts
rm -rf vendor/cef
rm -rf crates/arkavo-cef/cef-bridge/build

# Rebuild from scratch
cargo clean -p arkavo-cef
cargo build -p arkavo-cef
```

### Update CEF version

Edit `scripts/setup-cef.sh`:
```bash
CEF_VERSION="NEW_VERSION_HERE"
```

Then clean rebuild.

### Debug CMake issues

```bash
# Manual CMake build for debugging
cd crates/arkavo-cef/cef-bridge
mkdir -p build && cd build
cmake -DCEF_ROOT=../../../../vendor/cef -DCMAKE_VERBOSE_MAKEFILE=ON ..
cmake --build . --verbose
```

## See Also

- [CEF Project](https://bitbucket.org/chromiumembedded/cef/wiki/Home)
- [CEF Builds Download](https://cef-builds.spotifycdn.com/index.html)
- [CEF CMake Documentation](https://bitbucket.org/chromiumembedded/cef/wiki/LinkingCEFtoYourApplication)
