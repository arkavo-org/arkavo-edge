# Agent Prompt: Fix AXP Harness Compilation for iOS 26 Beta

You are tasked with fixing a compilation error in the AXP harness builder that occurs on iOS 26 beta. The issue is caused by SDK changes and symbol drift in the beta version.

## Your Mission

1. Read the implementation guide at: `/Users/paul/Projects/arkavo/arkavo-edge/AXP_HARNESS_FIX_GUIDE.md`
2. Follow the step-by-step instructions to update the code
3. Test the changes by building the project
4. Verify the fix works by running the test harness builder

## Key Files to Modify

1. `/Users/paul/Projects/arkavo/arkavo-edge/crates/arkavo-test/src/mcp/axp_harness_builder.rs`
   - Update the `compile_harness` method (starting at line 219)
   - Add new methods: `compile_with_sdk` and `compile_with_fallback`

2. `/Users/paul/Projects/arkavo/arkavo-edge/crates/arkavo-test/templates/ArkavoAXBridge.swift`
   - Add conditional imports for XCTest
   - Wrap XCTest-dependent methods with `#if canImport(XCTest)`

## Test Your Implementation

```bash
# Build the project
cargo build

# Run the test script if it exists
./test_axp_harness_build.sh

# Or test directly with MCP
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"build_test_harness","arguments":{"app_bundle_id":"com.test.app"}}}' | ./target/debug/arkavo-test-mcp
```

## Success Criteria

- [ ] Code compiles without errors
- [ ] AXP harness builder handles iOS 26 beta gracefully
- [ ] Falls back to iOS 18 target if iOS 26 compilation fails
- [ ] Provides clear error messages for debugging
- [ ] UI automation continues working even if AXP is unavailable

## Important Notes

- DO NOT remove existing functionality
- Ensure backward compatibility with iOS 18 and earlier
- The fix should be production-ready, not a prototype
- Test on both iOS 26 beta and stable iOS versions if possible

Begin by reading the implementation guide and understanding the changes needed.