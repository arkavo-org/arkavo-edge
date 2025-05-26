#!/bin/bash

echo "Testing Arkavo MCP Server run_test functionality"
echo "================================================"

# Function to call run_test
run_test() {
    local test_name=$1
    echo -e "\nRunning test: $test_name"
    echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"run_test\",\"arguments\":{\"test_name\":\"$test_name\"}}}" | \
    cargo run --bin arkavo -- serve 2>/dev/null | \
    tail -1 | jq -r '.result.content[0].text' | jq .
}

# Test the predefined test scenarios
run_test "eula_display"
run_test "registration_automation"
run_test "stream_creation_uniqueness"
run_test "biometric_enrollment"
run_test "system_dialog_handling"

# Test a dynamic test (will use the fallback runner)
run_test "custom_test_scenario"