#!/bin/bash

echo "=== Quick IDB Direct FFI Test ==="
echo

# Check Xcode path
XCODE_PATH=$(xcode-select -p)
echo "Current Xcode path: $XCODE_PATH"

if [[ ! "$XCODE_PATH" == *"Xcode"* ]]; then
    echo "⚠️  Xcode path doesn't look right. Please run:"
    echo "    sudo xcode-select -s /Applications/Xcode.app/Contents/Developer"
    exit 1
fi

# Build
echo
echo "Building project..."
cargo build --release || exit 1

# Test basic init
echo
echo "Testing basic initialization..."
cargo run --example simple_init || exit 1

# List simulators
echo
echo "Available simulators:"
xcrun simctl list devices available | grep -E "iPhone|iPad" | head -10

echo
echo "Next steps:"
echo "1. Boot a simulator:"
echo "   xcrun simctl boot <DEVICE-ID>"
echo
echo "2. Test connection:"
echo "   cargo run --example connect_test"
echo
echo "3. Test tap:"
echo "   cargo run --example tap_test"