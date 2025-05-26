#!/bin/bash

echo "Testing Arkavo MCP Server list_tests functionality"
echo "=================================================="

# Test list_tests
echo -e "\nListing all tests:"
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tests","arguments":{}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | \
tail -1 | jq -r '.result.content[0].text' | jq .

# Test list_tests with filter
echo -e "\nListing tests with 'test' filter:"
echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_tests","arguments":{"filter":"test"}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | \
tail -1 | jq -r '.result.content[0].text' | jq .

# Test running a discovered test
echo -e "\nRunning a test (example):"
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_test","arguments":{"test_name":"test_tool_discovery"}}}' | \
cargo run --bin arkavo -- serve 2>/dev/null | \
tail -1 | jq -r '.result.content[0].text' | jq .