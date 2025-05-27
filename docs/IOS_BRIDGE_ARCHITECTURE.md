# iOS Bridge Architecture

## Overview

The iOS bridge enables Arkavo's test harness to directly control and inspect iOS applications without going through the UI. This provides:
- Direct state manipulation (bypass UI delays)
- Deep inspection of app internals
- Snapshot/restore capabilities
- Sub-50ms test execution

## Architecture Layers

```
┌─────────────────────────────────────────────────────────────┐
│                    Arkavo Test Harness (Rust)                │
├─────────────────────────────────────────────────────────────┤
│                        FFI Bridge Layer                       │
│  - Type conversions (Rust ↔ C)                              │
│  - Memory management                                         │
│  - Error handling                                            │
├─────────────────────────────────────────────────────────────┤
│                    C Interface (ios_ffi.rs)                   │
│  - extern "C" function declarations                          │
│  - Raw pointer handling                                      │
│  - CString conversions                                       │
├─────────────────────────────────────────────────────────────┤
│                 iOS Bridge (Objective-C/Swift)                │
│  - XCTest private APIs                                       │
│  - Runtime manipulation                                      │
│  - App state access                                         │
├─────────────────────────────────────────────────────────────┤
│                     iOS Application                          │
└─────────────────────────────────────────────────────────────┘
```

## How It Works

### 1. Rust Side (ios_ffi.rs)

The Rust side defines:

```rust
// Opaque type representing iOS bridge
#[repr(C)]
pub struct IOSBridge {
    _private: [u8; 0],  // Zero-sized opaque type
}

// FFI function declarations
unsafe extern "C" {
    fn ios_bridge_execute_action(
        bridge: *mut IOSBridge,
        action: *const c_char,
        params: *const c_char
    ) -> *mut c_char;
    // ... more functions
}
```

### 2. C Interface

The bridge uses C as the common ABI between Rust and iOS:

```c
// Called from Rust
char* ios_bridge_execute_action(void* bridge, const char* action, const char* params) {
    // Cast to Objective-C object
    IOSTestBridge* testBridge = (__bridge IOSTestBridge*)bridge;
    
    // Convert C strings to NSString
    NSString* actionStr = [NSString stringWithUTF8String:action];
    NSString* paramsStr = [NSString stringWithUTF8String:params];
    
    // Execute action
    NSString* result = [testBridge executeAction:actionStr params:paramsStr];
    
    // Return as C string (caller must free)
    return strdup([result UTF8String]);
}
```

### 3. iOS Side (Objective-C/Swift)

The iOS implementation uses XCTest private APIs:

```objc
@interface IOSTestBridge : NSObject

- (NSString*)executeAction:(NSString*)action params:(NSString*)params {
    // Parse action and params
    NSDictionary* paramDict = [NSJSONSerialization JSONObjectWithData:...];
    
    if ([action isEqualToString:@"tap"]) {
        // Use XCTest private API to tap element
        XCUIElement* element = [self findElement:paramDict[@"id"]];
        [element tap];
    } else if ([action isEqualToString:@"setText"]) {
        // Direct text injection
        XCUIElement* textField = [self findElement:paramDict[@"id"]];
        [textField typeText:paramDict[@"text"]];
    }
    // ... more actions
}

- (NSString*)getCurrentState {
    // Access app's internal state directly
    UIApplication* app = [UIApplication sharedApplication];
    AppDelegate* delegate = (AppDelegate*)app.delegate;
    
    // Serialize current state
    NSDictionary* state = @{
        @"currentViewController": NSStringFromClass([self topViewController].class),
        @"userData": [delegate.currentUser toDictionary],
        @"navigationStack": [self getNavigationStack]
    };
    
    return [NSJSONSerialization dataWithJSONObject:state ...];
}

@end
```

### 4. Memory Management

The bridge carefully manages memory across FFI boundary:

```rust
// Rust side
let result_ptr = ios_bridge_execute_action(...);
let result = CStr::from_ptr(result_ptr).to_string_lossy().to_string();
ios_bridge_free_string(result_ptr);  // MUST free C string

// C side
void ios_bridge_free_string(char* s) {
    free(s);  // Free memory allocated by strdup
}
```

### 5. State Snapshots

The bridge can snapshot entire app state:

```objc
- (NSData*)createSnapshot {
    // Capture all relevant state
    NSMutableDictionary* snapshot = [NSMutableDictionary new];
    
    // UI state
    snapshot[@"viewHierarchy"] = [self captureViewHierarchy];
    
    // Data state
    snapshot[@"coreData"] = [self captureCoreDataState];
    snapshot[@"userDefaults"] = [[NSUserDefaults standardUserDefaults] dictionaryRepresentation];
    snapshot[@"keychain"] = [self captureKeychainItems];
    
    // Navigation state
    snapshot[@"navigationStack"] = [self captureNavigationState];
    
    return [NSKeyedArchiver archivedDataWithRootObject:snapshot];
}

- (void)restoreSnapshot:(NSData*)data {
    NSDictionary* snapshot = [NSKeyedUnarchiver unarchiveObjectWithData:data];
    
    // Restore in correct order
    [self restoreCoreDataState:snapshot[@"coreData"]];
    [self restoreUserDefaults:snapshot[@"userDefaults"]];
    [self restoreKeychain:snapshot[@"keychain"]];
    [self restoreNavigationState:snapshot[@"navigationStack"]];
    [self restoreViewHierarchy:snapshot[@"viewHierarchy"]];
}
```

## Building and Linking

### Development (Non-iOS)

For development on non-iOS platforms, we use C stubs:

```rust
// build.rs
fn main() {
    if !cfg!(target_os = "ios") {
        cc::Build::new()
            .file("src/bridge/ios_stub.c")
            .compile("ios_bridge_stub");
    }
}
```

### iOS Build

For actual iOS testing, link against the iOS framework:

```rust
// build.rs for iOS
fn main() {
    if cfg!(target_os = "ios") {
        println!("cargo:rustc-link-lib=framework=ArkavoTestBridge");
        println!("cargo:rustc-link-search=native={}/Frameworks", ios_sdk_path);
    }
}
```

## Usage Example

```rust
// Initialize bridge
let mut harness = RustTestHarness::new();
let bridge_ptr = create_ios_bridge(); // From iOS side
harness.connect_ios_bridge(bridge_ptr);

// Execute actions
let result = harness.execute_action("tap", r#"{"id": "loginButton"}"#)?;

// Create checkpoint
harness.checkpoint("before_login")?;

// Mutate state directly
harness.mutate_state("user", "login", r#"{"userId": "test123"}"#)?;

// Restore if test fails
harness.restore("before_login")?;
```

## Performance Benefits

1. **Direct State Access**: No UI automation delays
2. **Memory Snapshots**: Instant save/restore (vs. app restart)
3. **Parallel Execution**: Multiple snapshots = parallel test paths
4. **No Compilation**: Tests injected at runtime

## Security Considerations

- Bridge only works in test builds (XCTest environment)
- Requires test signing certificates
- Cannot access production apps
- Sandboxed to test environment

## Future Enhancements

1. **Record/Replay**: Record user sessions, replay with variations
2. **Fuzzing**: Automatic input generation based on UI structure
3. **Performance Profiling**: Track memory/CPU during tests
4. **Network Mocking**: Intercept and modify network calls
5. **Cross-App Testing**: Test app interactions (deep links, sharing)