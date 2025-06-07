#!/bin/bash
# Download idb_companion binaries for embedding in Arkavo

set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"
VENDOR_DIR="$PROJECT_ROOT/crates/arkavo-test/vendor"

echo "Setting up idb_companion for Arkavo..."

# Create vendor directory
mkdir -p "$VENDOR_DIR"

# Detect architecture
ARCH=$(uname -m)
if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
    ARCH_SUFFIX="arm64"
elif [ "$ARCH" = "x86_64" ]; then
    ARCH_SUFFIX="x86_64"
else
    echo "Unsupported architecture: $ARCH"
    exit 1
fi

echo "Detected architecture: $ARCH_SUFFIX"

# Download URL for idb_companion
# Note: You'll need to update this with the actual download URL
# This is a placeholder showing the expected format
IDB_VERSION="latest"
DOWNLOAD_URL="https://github.com/facebook/idb/releases/download/$IDB_VERSION/idb_companion_macos_$ARCH_SUFFIX"

echo "Downloading idb_companion..."
curl -L "$DOWNLOAD_URL" -o "$VENDOR_DIR/idb_companion_$ARCH_SUFFIX"

# Make executable
chmod +x "$VENDOR_DIR/idb_companion_$ARCH_SUFFIX"

echo "Downloaded to: $VENDOR_DIR/idb_companion_$ARCH_SUFFIX"

# Verify the binary works
if "$VENDOR_DIR/idb_companion_$ARCH_SUFFIX" --version; then
    echo "✅ idb_companion is working correctly"
else
    echo "❌ idb_companion binary verification failed"
    exit 1
fi

echo ""
echo "To embed idb_companion in your Arkavo build:"
echo "1. Update crates/arkavo-test/build.rs to use use_predownloaded_binary()"
echo "2. Run: cargo build -p arkavo-test"
echo ""
echo "The binary will be embedded in the Arkavo executable."