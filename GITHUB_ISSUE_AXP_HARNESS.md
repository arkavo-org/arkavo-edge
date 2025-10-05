# GitHub Issue: Implement UI Test Harness for AXP Translator Touch Injection

## Summary
Implement a UI test harness that leverages Apple's signed UI Test Runner to access private AXPTranslator APIs for touch injection on Xcode 16+, while maintaining HID fallback for compatibility.

## Background
Xcode 16+ removed legacy touch injection APIs and requires using `sendAccessibilityRequestAsync` with AXPTranslator. These APIs require private entitlements (`com.apple.private.axplatformtranslator`) that only Apple-signed binaries possess. The solution is to run our code inside Apple's UI Test Runner which already has these entitlements.

## Technical Approach
1. Create a minimal UI test target that runs inside Apple's signed runner
2. Expose a Unix domain socket (`/tmp/arkavo_tap.sock`) for IPC
3. Route touch commands through the harness when available
4. Fall back to HID when harness is unavailable

## Work Items

### 1. Create UITapHarness Xcode Project ⬜
**Owner**: iOS team  
**Done means**: `UITapHarness.xcodeproj` builds & `xcodebuild test-with-ui-testing` succeeds locally

**Deliverables**:
- New Xcode project with UI test target only
- Minimal Swift test that keeps runner alive
- Project structure under `ios/UITapHarness/`

### 2. Implement Socket Bridge Layer ⬜
**Owner**: iOS team  
**Done means**: Receiving JSON `{"tap":[x,y]}` produces visible tap in simulator

**Requirements**:
- Unix socket server at `/tmp/arkavo_tap.sock`
- JSON protocol for commands
- Swift implementation using `sendTapWithAXP()`
- Single file, <300 LOC

### 3. Rust CLI Auto-Detection ⬜
**Owner**: Core team  
**Done means**: `arkavo edge tap 100 200` uses AXP when harness active, HID otherwise

**Implementation**:
```rust
if std::fs::metadata("/tmp/arkavo_tap.sock").is_ok() {
    send_to_harness(command)
} else {
    tap_via_hid(x, y)
}
```

### 4. Runtime Entitlement Probes ⬜
**Owner**: iOS team  
**Done means**: Harness falls back to HID if private entitlements disappear

**Guards needed**:
- `AXTranslatorAvailable()`
- `hasAXPEntitlements()`
- Defensive fallback implementation

### 5. CI Job Configuration ⬜
**Owner**: DevOps  
**Done means**: CI fails if either touch path stops working

**CI job "macos-uitest"**:
- Bootstrap iOS 16 simulator
- Run `xcodebuild test-with-ui-testing` to launch harness
- Execute sample tap via CLI
- Assert SpringBoard PID changes

### 6. Documentation ⬜
**Owner**: Docs team  
**Done means**: `docs/sim-touch-injection.md` explains dual-path approach

**Content**:
- Why two paths exist
- How to debug issues
- Socket regeneration
- Entitlement requirements

## Out of Scope
- ❌ Real device support (runner lacks entitlements on device)
- ❌ Gestures beyond tap/swipe (scroll, pinch, hardware buttons)
- ❌ Notarization/distribution of pre-built harness

## Quick Test Commands

```bash
# Verify harness has entitlements
log show --last 1m --predicate 'senderImagePath contains "amfid"'

# Check if socket exists
ls -la /tmp/arkavo_tap.sock

# Verify harness process sandbox
launchctl print gui/$UID/$(pgrep UITapHarness)

# Test HID fallback (should fail on Xcode 17+)
xcrun simctl io booted tap 10 10
```

## Recommended PR Structure
1. `ios/UITapHarness/` - New Xcode project (100% Swift)
2. `crates/arkavo-test/src/ios/harness_bridge.rs` - Rust socket client
3. `Makefile` target `test-uitap` - Build and test harness
4. CI: Add `jobs.harness-test` using Makefile target
5. Documentation updates

**Each PR component should be ≤300 LOC for efficient review**

## Go/No-Go Checklist
- [ ] Tap lands in SpringBoard when harness socket is open
- [ ] Same command lands via HID when socket absent
- [ ] CI green on both paths
- [ ] Documentation explains entitlement caveats

## Risk Mitigation
The primary risk is Apple revoking private entitlements from the template runner in future Xcode versions. Our runtime probes (item #4) and dual-job CI (item #5) will catch this immediately when new betas release.

## Success Criteria
- Touch injection works on Xcode 16+ via AXPTranslator
- Automatic fallback to HID maintains compatibility
- No manual configuration required by users
- CI validates both paths continuously

## Labels
- `enhancement`
- `ios`
- `testing`
- `xcode-16-compat`
- `priority-high`