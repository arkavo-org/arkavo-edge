#!/bin/bash

echo "=== iOS 26 Beta AXP Symbol Discovery ==="
echo ""

# Find the simulator runtime path
RUNTIME_PATH=$(xcrun simctl list runtimes -j | jq -r '.runtimes[] | select(.identifier | contains("iOS-26")) | .bundlePath' | head -1)

if [ -z "$RUNTIME_PATH" ]; then
    echo "❌ iOS 26 runtime not found"
    echo "Available runtimes:"
    xcrun simctl list runtimes
    exit 1
fi

echo "Found iOS 26 runtime: $RUNTIME_PATH"
echo ""

# Function to check framework for symbols
check_framework() {
    local framework_path="$1"
    local framework_name=$(basename "$framework_path")
    
    echo "Checking $framework_name..."
    
    if [ -f "$framework_path" ]; then
        # Use nm to list symbols
        nm -gU "$framework_path" 2>/dev/null | grep -E "(AX|CS|Accessibility)" | grep -E "(Perform|HitTest|Copy|Touch|Tap|Event)" | head -20
        
        # Also try strings for Swift mangled names
        echo ""
        echo "Swift symbols:"
        strings "$framework_path" | grep -E "AccessibilityPlatformTranslation" | head -10
    else
        echo "  ❌ Framework not found at path"
    fi
    echo ""
}

# Check various framework locations
FRAMEWORKS=(
    "$RUNTIME_PATH/Contents/Resources/RuntimeRoot/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework/AccessibilityPlatformTranslation"
    "$RUNTIME_PATH/Contents/Resources/RuntimeRoot/System/Library/PrivateFrameworks/AccessibilityUtilities.framework/AccessibilityUtilities"
    "$RUNTIME_PATH/Contents/Resources/RuntimeRoot/System/Library/PrivateFrameworks/AccessibilityUIUtilities.framework/AccessibilityUIUtilities"
    "$RUNTIME_PATH/Contents/Resources/RuntimeRoot/System/Library/PrivateFrameworks/AXRuntime.framework/AXRuntime"
)

for framework in "${FRAMEWORKS[@]}"; do
    check_framework "$framework"
done

echo "=== Recommendations ==="
echo "1. Add any discovered symbols to ArkavoAXBridge.swift"
echo "2. Build against iOS 26 SDK: xcrun --sdk iphonesimulator26.0 --show-sdk-path"
echo "3. Keep the fallback path for when symbols aren't found"