#!/bin/bash

echo "=== IDB Companion Verification Script ==="
echo

# Find the IDB binary
IDB_PATH="../../target/arkavo_idb/bin/idb_companion"

if [ ! -f "$IDB_PATH" ]; then
    echo "IDB companion not found at expected location: $IDB_PATH"
    echo "Running cargo build to extract it..."
    cargo build
fi

if [ -f "$IDB_PATH" ]; then
    echo "Found IDB companion at: $IDB_PATH"
    echo
    
    # Check signature
    echo "1. Checking code signature:"
    codesign -dv "$IDB_PATH" 2>&1 | grep -E "TeamIdentifier|Identifier|Timestamp"
    echo
    
    # Check extended attributes
    echo "2. Checking extended attributes:"
    xattr -l "$IDB_PATH" 2>/dev/null || echo "No extended attributes"
    echo
    
    # Try to run with --help
    echo "3. Testing basic execution (--help):"
    "$IDB_PATH" --help >/dev/null 2>&1
    if [ $? -eq 0 ]; then
        echo "✓ Basic execution works"
    else
        echo "✗ Basic execution failed with exit code: $?"
    fi
    echo
    
    # Try to list targets
    echo "4. Testing target listing:"
    "$IDB_PATH" --list 1 --json 2>&1 | head -5
    EXIT_CODE=$?
    echo "Exit code: $EXIT_CODE"
    echo
    
    # Check if it can start as companion
    echo "5. Testing companion mode (will timeout after 3 seconds):"
    timeout 3 "$IDB_PATH" --udid "test-device" --only simulator 2>&1 | head -10
    EXIT_CODE=$?
    if [ $EXIT_CODE -eq 124 ]; then
        echo "✓ Companion mode started (timed out as expected)"
    elif [ $EXIT_CODE -eq 137 ] || [ $EXIT_CODE -eq 9 ]; then
        echo "✗ SIGKILL detected - macOS security is blocking execution"
    else
        echo "Exit code: $EXIT_CODE"
    fi
else
    echo "ERROR: Could not find or build IDB companion"
fi

echo
echo "=== Verification Complete ==="