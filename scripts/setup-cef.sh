#!/bin/bash
set -euo pipefail

# CEF version to download (latest stable as of 2025-10-13)
CEF_VERSION="138.0.51+g41d93d2+chromium-138.0.7204.293"
# Detect platform architecture
if [ "$(uname -m)" = "arm64" ]; then
    CEF_PLATFORM="macosarm64"
else
    CEF_PLATFORM="macosx64"
fi

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
    curl -L --fail --progress-bar -o "$CEF_FILENAME" "$CEF_URL"
    if [ $? -ne 0 ]; then
        echo "Error: Download failed with curl"
        rm -f "$CEF_FILENAME"
        exit 1
    fi
elif command -v wget &> /dev/null; then
    wget --progress=bar:force -O "$CEF_FILENAME" "$CEF_URL"
    if [ $? -ne 0 ]; then
        echo "Error: Download failed with wget"
        rm -f "$CEF_FILENAME"
        exit 1
    fi
else
    echo "Error: Neither curl nor wget found. Please install one to continue."
    exit 1
fi

# Verify download succeeded
if [ ! -s "$CEF_FILENAME" ]; then
    echo "Error: Downloaded file is empty or missing"
    ls -lh "$CEF_FILENAME" || true
    exit 1
fi

echo "    Downloaded: $(du -h "$CEF_FILENAME" | cut -f1)"

echo "==> Extracting CEF..."
tar -xjf "$CEF_FILENAME"

# Find the extracted directory (might have slight variations in naming)
EXTRACTED_DIR=$(find . -maxdepth 1 -type d -name "cef_binary_*_${CEF_PLATFORM}" | head -n 1)
if [ -n "$EXTRACTED_DIR" ] && [ -d "$EXTRACTED_DIR" ]; then
    mv "$EXTRACTED_DIR" cef
    echo "==> CEF extracted to $CEF_DIR"
else
    echo "Error: Could not find extracted CEF directory matching pattern: cef_binary_*_${CEF_PLATFORM}"
    echo "Available directories:"
    ls -la
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
