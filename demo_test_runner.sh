#!/bin/bash

echo "Arkavo MCP Test Runner Demo"
echo "==========================="
echo ""
echo "This demonstrates the enhanced run_test functionality that:"
echo "- Discovers tests from the local repository"
echo "- Runs tests at multiple levels (unit, integration, performance)"
echo "- Supports multiple languages (Rust, Swift, JavaScript, Python, Go)"
echo ""

# List available tests
echo "1. Listing all available tests in the repository:"
echo "================================================"
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tests","arguments":{}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | \
tail -1 | jq -r '.result.content[0].text' | jq .

echo ""
echo "2. Running a specific test:"
echo "=========================="
echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_test","arguments":{"test_name":"test_tool_discovery"}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | \
tail -1 | jq -r '.result.content[0].text' | jq '{test_name, status, test_type, duration_ms}'

echo ""
echo "3. List only unit tests:"
echo "======================="
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_tests","arguments":{"test_type":"unit"}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | \
tail -1 | jq -r '.result.content[0].text' | jq '.count'

echo ""
echo "Key Features:"
echo "- Automatic project type detection (Rust, Swift, JS, Python, Go)"
echo "- Test categorization (unit, integration, performance)"
echo "- Real test execution with proper output capture"
echo "- Error detection and reporting"
echo "- Timeout handling for long-running tests"