/// iOS 26 beta compilation guidance constants
pub(crate) const IOS26_BETA_COMPILATION_GUIDANCE: &str = r#"
## iOS 26 Beta Compilation Fix

### Problem
When building on iOS 26 beta simulators, XCTest framework symbols may be missing or incompatible due to SDK/runtime mismatch.

### Symptoms
- Compilation errors: "SDK does not contain 'XCTest.framework'"
- Symbol errors: "cannot find type 'XCUICoordinate' in scope"
- Runtime crashes when trying to use AXP functions

### Root Cause
Apple's beta releases often have symbol drift between the SDK and runtime. The XCTest framework in iOS 26 beta may have changed internal symbols that AXP relies on.

### Solution Strategy
1. **Minimal Mode**: Use templates without XCTest dependency
2. **Fallback Compilation**: Target iOS 18.0 but use iOS 26 SDK
3. **Runtime Detection**: Check for iOS 26 beta and switch to IDB/AppleScript

### Implementation
The harness builder automatically:
1. Detects iOS 26 beta from device runtime
2. Uses minimal templates (ArkavoAXBridgeMinimal.swift)
3. Compiles without XCTest framework
4. Falls back to IDB for actual touch injection

### Performance Impact
- AXP taps: <30ms (not available on iOS 26 beta)
- IDB taps: ~100ms (fallback for iOS 26 beta)
- AppleScript: ~200ms (last resort)

### Recommendations
1. Use iOS 18 simulators for testing when possible
2. Wait for stable iOS 26 release
3. Install matching Xcode 16 beta with iOS 26 SDK
4. Use IDB (idb_companion) for iOS 26 beta automation
"#;

/// Error codes for AXP harness operations
pub(crate) mod error_codes {
    pub(crate) const IOS_SDK_MISSING: &str = "IOS_SDK_MISSING";
    pub(crate) const INVALID_BUNDLE_ID: &str = "INVALID_BUNDLE_ID";
    pub(crate) const COMPILATION_FAILED: &str = "COMPILATION_FAILED";
    pub(crate) const BETA_COMPILATION_FAILED: &str = "BETA_COMPILATION_FAILED";
    pub(crate) const SDK_NOT_FOUND: &str = "SDK_NOT_FOUND";
}
