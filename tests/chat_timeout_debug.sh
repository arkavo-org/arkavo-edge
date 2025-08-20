#!/bin/bash

# Test script for debugging chat command timeout issue
# GitHub Issue: https://github.com/arkavo-org/arkavo-edge/issues/207
# 
# Issue: Chat command times out after model load without responding
# This script runs the chat command with enhanced debug logging to help
# identify where the hang occurs in the response generation pipeline

echo "Testing chat command with enhanced debugging..."
echo "Building debug version with logging enabled..."

# Build with debug logging
cargo build -p arkavo-cli -p arkavo-llm 2>&1 | tail -5

echo ""
echo "Running chat command with timeout and debug output..."
echo "Command: RUST_LOG=arkavo_llm=debug,arkavo_cli=debug ARKAVO_NO_TERMINAL_RELAUNCH=1 timeout 10 ./target/debug/arkavo chat --prompt \"What is 2+2?\""
echo ""

# Run the command with enhanced logging
RUST_LOG=arkavo_llm=debug,arkavo_cli=debug ARKAVO_NO_TERMINAL_RELAUNCH=1 timeout 10 ./target/debug/arkavo chat --prompt "What is 2+2?"

EXIT_CODE=$?

echo ""
echo "Test completed with exit code: $EXIT_CODE"

if [ $EXIT_CODE -eq 124 ]; then
    echo "ERROR: Command timed out after 10 seconds"
    echo "This confirms the issue described in #207"
    echo ""
    echo "Check the debug output above to identify where the hang occurs:"
    echo "- Look for [DEBUG] messages showing the last successful operation"
    echo "- Check if model loading completed"
    echo "- See if stream creation succeeded"
    echo "- Verify if any tokens were generated"
else
    echo "SUCCESS: Command completed without timeout"
fi

exit $EXIT_CODE