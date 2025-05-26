#!/bin/bash

echo "Testing Swift test discovery"
echo "==========================="

# Create a temporary Swift test file structure
TEMP_DIR=$(mktemp -d)
cd "$TEMP_DIR"

# Create a simple Swift test file
mkdir -p Tests
cat > Tests/ExampleTests.swift << 'EOF'
import XCTest

class ExampleTests: XCTestCase {
    func testExample() {
        XCTAssertEqual(1, 1)
    }
    
    func testAnother() {
        XCTAssertTrue(true)
    }
}

class ExampleUITests: XCTestCase {
    func testUIFlow() {
        // UI test
    }
}
EOF

# Create an xcodeproj marker
mkdir Example.xcodeproj

# Test list_tests
echo "Testing list_tests on Swift project:"
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tests","arguments":{}}}' | \
cargo run --bin arkavo --manifest-path "$OLDPWD/Cargo.toml" -- serve 2>/dev/null | \
tail -1 | jq -r '.result.content[0].text' | jq .

# Cleanup
cd "$OLDPWD"
rm -rf "$TEMP_DIR"