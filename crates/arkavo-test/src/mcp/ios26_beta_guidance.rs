use super::embedded_ios26_fix;
use super::server::{Tool, ToolSchema};
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;

/// MCP tool to provide iOS 26 beta compilation guidance
pub struct Ios26BetaGuidance {
    schema: ToolSchema,
}

impl Default for Ios26BetaGuidance {
    fn default() -> Self {
        Self {
            schema: ToolSchema {
                name: "ios26_beta_guidance".to_string(),
                description: "Get guidance for handling iOS 26 beta compilation issues with XCTest framework. This provides embedded knowledge about workarounds and solutions for beta SDK problems.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "topic": {
                            "type": "string",
                            "enum": ["compilation", "symbols", "workarounds", "embedded", "all"],
                            "default": "all",
                            "description": "Specific topic to get guidance on (embedded shows the complete fix documentation)"
                        }
                    }
                }),
            },
        }
    }
}

impl Ios26BetaGuidance {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Tool for Ios26BetaGuidance {
    async fn execute(&self, params: Value) -> Result<Value> {
        let topic = params
            .get("topic")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        let guidance = match topic {
            "compilation" => COMPILATION_GUIDANCE,
            "symbols" => SYMBOLS_GUIDANCE,
            "workarounds" => WORKAROUNDS_GUIDANCE,
            "embedded" => &embedded_ios26_fix::get_all_documentation(),
            "all" => ALL_GUIDANCE,
            _ => "Unknown topic. Use: compilation, symbols, workarounds, embedded, or all",
        };

        Ok(serde_json::json!({
            "success": true,
            "topic": topic,
            "guidance": guidance,
            "embedded_fix": {
                "status": "ACTIVE",
                "version": embedded_ios26_fix::IOS26_BETA_FIX_VERSION,
                "description": "The iOS 26 beta fix is fully embedded in this binary",
                "features": [
                    "Automatic iOS 26 beta detection from device runtime",
                    "Minimal templates compiled in via include_str!",
                    "Three-strategy compilation fallback system",
                    "IDB automatic fallback for touch events",
                    "No external files or manual fixes needed"
                ],
                "implementation": {
                    "detection": "axp_harness_builder.rs:124-129",
                    "templates": "axp_harness_builder.rs:134-156",
                    "compilation": "axp_harness_builder.rs:463-551",
                    "guidance": "ios26_beta_guidance.rs + embedded_ios26_fix.rs"
                },
                "usage": "Just run the tool normally - iOS 26 beta is handled automatically!"
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// Embedded guidance constants
const COMPILATION_GUIDANCE: &str = r#"
## iOS 26 Beta Compilation Issues

### Problem
XCTest.framework may be missing or have incompatible symbols in iOS 26 beta SDKs.

### Error Messages
- "SDK does not contain 'XCTest.framework'"
- "No such module 'XCTest'"
- "Could not find module 'XCTest' for target 'arm64-apple-ios26.0-simulator'"

### Solution
The harness builder automatically:
1. Detects iOS 26 beta from device.runtime string
2. Uses ArkavoAXBridgeMinimal.swift (no XCTest dependency)
3. Compiles with -target arm64-apple-ios18.0-simulator
4. Defines IOS_26_BETA compile flag

### Code Location
See axp_harness_builder.rs:
- Line 124-129: iOS 26 beta detection
- Line 134-141: Minimal template selection
- Line 419-482: Fallback compilation logic
"#;

const SYMBOLS_GUIDANCE: &str = r#"
## iOS 26 Beta Symbol Issues

### Missing Symbols
- XCUICoordinate (from XCTest framework)
- AXPTranslationLayerHelper (private API)
- AXPTranslatorRequest/Response types

### Symbol Drift
Beta releases often have:
- Changed function signatures
- Renamed internal symbols
- Modified framework dependencies

### Mitigation
The minimal templates avoid these symbols by:
- Not importing XCTest
- Using direct UIKit methods
- Returning stub responses for AXP calls
- Delegating to IDB for actual automation
"#;

const WORKAROUNDS_GUIDANCE: &str = r#"
## iOS 26 Beta Workarounds

### 1. Use Minimal Templates
- ArkavoAXBridgeMinimal.swift: No XCTest dependency
- ArkavoTestRunnerMinimal.swift: Basic socket server only

### 2. Fallback Compilation
```swift
swiftc -sdk /path/to/sdk \
       -target arm64-apple-ios18.0-simulator \  # iOS 18 target
       -D IOS_26_BETA \                        # Beta flag
       -framework Foundation \
       -framework CoreGraphics
```

### 3. Runtime Alternatives
- IDB (idb_companion): ~100ms taps, reliable
- AppleScript: ~200ms taps, last resort
- Direct HID events: Requires entitlements

### 4. Detection Logic
```rust
let is_ios26_beta = device.runtime.contains("iOS-26");
if is_ios26_beta {
    // Use minimal templates
    // Skip XCTest framework
    // Fall back to IDB
}
```
"#;

const ALL_GUIDANCE: &str = r#"
## Complete iOS 26 Beta Guidance

### Overview
iOS 26 beta has XCTest framework compatibility issues that prevent normal AXP compilation.
This is automatically handled by the embedded fix in axp_harness_builder.rs.

### How It Works

1. **Detection** (lines 124-129)
   - Checks device.runtime for "iOS-26"
   - Sets is_ios26_beta flag

2. **Template Selection** (lines 134-156)
   - iOS 26: ArkavoAXBridgeMinimal.swift
   - Others: ArkavoAXBridge.swift

3. **Compilation** (lines 370-382)
   - iOS 26: Skip -framework XCTest
   - iOS 26: Skip XCTest library paths

4. **Fallback** (lines 419-482)
   - Try iOS 18 target with iOS 26 SDK
   - Define IOS_26_BETA flag
   - Minimal framework dependencies

5. **Runtime** (lines 229-275)
   - Report iOS 26 beta in capabilities
   - Recommend IDB for automation
   - Warn about performance impact

### Best Practices

1. **Development**
   - Test on iOS 18 simulators when possible
   - Use IDB for iOS 26 beta automation
   - Monitor Xcode beta releases

2. **CI/CD**
   - Pin to stable iOS versions
   - Add iOS 26 beta as experimental
   - Expect slower test execution

3. **Production**
   - Wait for stable iOS 26 release
   - Keep fallback mechanisms
   - Log beta detection for debugging

### Performance Comparison
| Method | iOS 18 | iOS 26 Beta |
|--------|---------|-------------|
| AXP    | <30ms   | N/A         |
| IDB    | ~100ms  | ~100ms      |
| Script | ~200ms  | ~200ms      |

### Embedded Implementation
The fix is compiled into the binary at:
- Templates: src/mcp/templates.rs
- Builder: src/mcp/axp_harness_builder.rs
- No external files needed!
"#;
