#!/bin/bash
set -euo pipefail

# CEF version to download
CEF_VERSION="131.2.7+g872dfbe+chromium-131.0.6778.86"
CEF_PLATFORM="macosx64"

VENDOR_DIR="$(cd "$(dirname "$0")/.." && pwd)/vendor"
CEF_DIR="$VENDOR_DIR/cef"

echo "==> Setting up CEF for Arkavo Edge"
echo "    Version: $CEF_VERSION"
echo "    Platform: $CEF_PLATFORM"
echo "    Target: $CEF_DIR"

if [ -d "$CEF_DIR" ] && [ -f "$CEF_DIR/include/cef_version.h" ]; then
    echo "==> CEF already installed at $CEF_DIR"
    echo "    To reinstall, remove: rm -rf $CEF_DIR"
    exit 0
fi

mkdir -p "$VENDOR_DIR"
cd "$VENDOR_DIR"

CEF_FILENAME="cef_binary_${CEF_VERSION}_${CEF_PLATFORM}.tar.bz2"
CEF_URL="https://cef-builds.spotifycdn.com/$CEF_FILENAME"

echo "==> Downloading CEF..."
echo "    URL: $CEF_URL"

if command -v curl &> /dev/null; then
    curl -L --progress-bar -o "$CEF_FILENAME" "$CEF_URL"
elif command -v wget &> /dev/null; then
    wget --progress=bar:force -O "$CEF_FILENAME" "$CEF_URL"
else
    echo "Error: Neither curl nor wget found. Please install one to continue."
    exit 1
fi

echo "==> Extracting CEF..."
tar -xjf "$CEF_FILENAME"

EXTRACTED_DIR="cef_binary_${CEF_VERSION}_${CEF_PLATFORM}"
if [ -d "$EXTRACTED_DIR" ]; then
    mv "$EXTRACTED_DIR" cef
    echo "==> CEF extracted to $CEF_DIR"
else
    echo "Error: Expected directory $EXTRACTED_DIR not found after extraction"
    exit 1
fi

echo "==> Cleaning up archive..."
rm -f "$CEF_FILENAME"

echo "==> Building CEF DLL wrapper..."
cd "$CEF_DIR"
mkdir -p build_wrapper
cd build_wrapper

cmake -DCMAKE_BUILD_TYPE=Release ..
cmake --build . --target libcef_dll_wrapper --config Release -- -j$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)

echo "==> CEF setup complete!"
echo "    Location: $CEF_DIR"
echo "    Size: $(du -sh "$CEF_DIR" | cut -f1)"
