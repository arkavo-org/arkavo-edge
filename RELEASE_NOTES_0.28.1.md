## Arkavo Edge 0.28.1

Patch release fixing Windows build issues.

### Installation

Download the appropriate binary for your platform and place it in your PATH.

### Binaries

#### Linux
- **Full featured** (glibc, local LLM + remote LLM + mDNS): arkavo-0.28.1-x86_64-linux.tar.gz
- **Static/slim** (musl, memory + mDNS): arkavo-0.28.1-x86_64-linux-musl.tar.gz
- **Debian package** (full featured): arkavo_0.28.1_amd64.deb

#### macOS
- **ARM64** (full featured): arkavo-0.28.1-aarch64-apple-darwin.tar.gz

#### Windows
- **x86_64** (memory + remote LLM): arkavo-0.28.1-x86_64-windows.zip

Choose the musl build for containers/Alpine Linux with remote LLM support. Choose the glibc build for desktop Linux with both local and remote LLM support. Windows builds include memory and remote LLM support (iOS testing capabilities are not available on Windows).

### What's Changed

#### Bug Fixes
* Fix Windows build CMake installation error by @arkavo-com in https://github.com/arkavo-org/arkavo-edge/pull/225
  - Fixed refreshenv error in PowerShell during CMake installation
  - Windows x86_64 release builds now complete successfully

#### Technical Details
- Fixed PowerShell compatibility issue with Chocolatey's refreshenv command
- Updated llama.cpp submodule to latest upstream commit

**Full Changelog**: https://github.com/arkavo-org/arkavo-edge/compare/0.28.0...0.28.1