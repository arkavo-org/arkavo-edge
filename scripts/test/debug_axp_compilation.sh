#!/bin/bash

echo "=== AXP Harness Compilation Debug Script ==="
echo ""

# 1. Check Xcode installation
echo "1. Xcode Version:"
if command -v xcodebuild &> /dev/null; then
    xcodebuild -version
else
    echo "ERROR: xcodebuild not found!"
fi
echo ""

# 2. Check xcode-select
echo "2. Xcode Developer Directory:"
if command -v xcode-select &> /dev/null; then
    xcode-select -p
else
    echo "ERROR: xcode-select not found!"
fi
echo ""

# 3. List available SDKs
echo "3. Available SDKs:"
if command -v xcodebuild &> /dev/null; then
    xcodebuild -showsdks | grep -i simulator || echo "No simulator SDKs found"
else
    echo "Cannot list SDKs - xcodebuild not available"
fi
echo ""

# 4. Check specific SDK paths
echo "4. SDK Paths:"
echo -n "Default iphonesimulator SDK: "
xcrun --sdk iphonesimulator --show-sdk-path 2>/dev/null || echo "Not found"

echo -n "iOS 26 SDK: "
xcrun --sdk iphonesimulator26.0 --show-sdk-path 2>/dev/null || echo "Not found"
echo ""

# 5. Check Swift version
echo "5. Swift Version:"
if command -v swiftc &> /dev/null; then
    swiftc --version
else
    echo "ERROR: swiftc not found!"
fi
echo ""

# 6. Check simulator runtimes
echo "6. Available Simulator Runtimes:"
xcrun simctl list runtimes 2>/dev/null | grep iOS || echo "No iOS runtimes found"
echo ""

# 7. Check for XCTest framework
echo "7. XCTest Framework Locations:"
SDK_PATH=$(xcrun --sdk iphonesimulator --show-sdk-path 2>/dev/null)
if [ -n "$SDK_PATH" ]; then
    echo "Checking in SDK: $SDK_PATH"
    find "$SDK_PATH" -name "XCTest.framework" -type d 2>/dev/null | head -5 || echo "XCTest.framework not found in SDK"
    
    # Also check developer frameworks
    DEV_PATH="/Applications/Xcode.app/Contents/Developer"
    echo ""
    echo "Checking in Developer Directory:"
    find "$DEV_PATH/Platforms/iPhoneSimulator.platform" -name "XCTest.framework" -type d 2>/dev/null | head -5 || echo "XCTest.framework not found in Developer dir"
fi
echo ""

# 8. Test minimal compilation
echo "8. Testing Minimal Swift Compilation:"
cat > /tmp/test_minimal.swift << 'EOF'
import Foundation

@objc public class TestMinimal: NSObject {
    @objc public func test() -> String {
        return "Minimal compilation works"
    }
}
EOF

echo "Compiling minimal Swift file..."
if [ -n "$SDK_PATH" ]; then
    swiftc -sdk "$SDK_PATH" \
           -target arm64-apple-ios15.0-simulator \
           -parse-as-library \
           -emit-library \
           -module-name TestMinimal \
           -o /tmp/test_minimal \
           /tmp/test_minimal.swift 2>&1
    
    if [ $? -eq 0 ]; then
        echo "✅ Minimal compilation succeeded"
    else
        echo "❌ Minimal compilation failed"
    fi
else
    echo "Cannot test compilation - SDK path not found"
fi
echo ""

# 9. Test with XCTest
echo "9. Testing XCTest Compilation:"
cat > /tmp/test_xctest.swift << 'EOF'
import Foundation
#if canImport(XCTest)
import XCTest
@objc public class TestXCTest: NSObject {
    @objc public func hasXCTest() -> Bool {
        return true
    }
}
#else
@objc public class TestXCTest: NSObject {
    @objc public func hasXCTest() -> Bool {
        return false
    }
}
#endif
EOF

echo "Compiling with XCTest..."
if [ -n "$SDK_PATH" ]; then
    swiftc -sdk "$SDK_PATH" \
           -target arm64-apple-ios15.0-simulator \
           -parse-as-library \
           -emit-library \
           -module-name TestXCTest \
           -framework XCTest \
           -F "$SDK_PATH/../../Library/Frameworks" \
           -o /tmp/test_xctest \
           /tmp/test_xctest.swift 2>&1
    
    if [ $? -eq 0 ]; then
        echo "✅ XCTest compilation succeeded"
    else
        echo "❌ XCTest compilation failed - this is expected for iOS 26 beta"
    fi
else
    echo "Cannot test compilation - SDK path not found"
fi
echo ""

echo "=== Debug script complete ==="
echo ""
echo "If you see compilation failures above, try:"
echo "1. Install Xcode from the App Store"
echo "2. Run: xcode-select --install"
echo "3. Run: sudo xcode-select --switch /Applications/Xcode.app"
echo "4. For iOS 26 beta: Install Xcode 16 beta"