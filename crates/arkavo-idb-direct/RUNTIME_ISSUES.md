# IDB Direct FFI Runtime Issues

## NSInvalidArgumentException with SimDeviceSet

### Issue
When calling `idb_connect_target`, the following exception occurs:
```
*** Terminating app due to uncaught exception 'NSInvalidArgumentException', 
reason: '+[SimDeviceSet defaultSet]: unrecognized selector sent to class'
```

### Root Cause
This is a **CoreSimulator API change** in Xcode 16+, not a static linking issue:

- **Xcode ≤ 15.x**: CoreSimulator exported `+[SimDeviceSet defaultSet]`
- **Xcode 16+**: Apple removed this selector. The new API path uses `SimServiceContext`

The static library was compiled against an older Xcode version and calls the now-removed selector.

### Current Status
- ✅ `idb_initialize()` - Works correctly
- ✅ `idb_version()` - Works correctly  
- ✅ `idb_shutdown()` - Works correctly
- ❌ `idb_connect_target()` - Calls obsolete API (`+[SimDeviceSet defaultSet]`)
- ❌ `idb_tap()` - Requires connected target
- ❌ `idb_take_screenshot()` - Requires connected target

### Solution
The Objective-C code needs to be updated to support both API versions:

```objc
// Runtime detection of available API
Class SSC = NSClassFromString(@"SimServiceContext");
if ([SSC respondsToSelector:@selector(sharedServiceContext)]) {
    // Xcode 16+ path
    id ctx = [SSC sharedServiceContext];
    id deviceSet = [ctx defaultDeviceSet];
    // ... use deviceSet
} else {
    // Legacy path for Xcode 15 and earlier
    Class SDS = NSClassFromString(@"SimDeviceSet");
    if ([SDS respondsToSelector:@selector(defaultSet)]) {
        id deviceSet = [SDS defaultSet];
        // ... use deviceSet
    }
}
```

### Workarounds

1. **Use IDB Companion Fallback**: The unified wrapper automatically falls back to the traditional IDB companion which may have been compiled with a compatible Xcode version

2. **Use FBControlCore**: Meta's FBControlCore library maintains compatibility across Xcode versions

### Impact
Until the static library is recompiled with Xcode version compatibility, Direct FFI only supports initialization and version checking. Device operations will throw NSException.

### Next Steps
1. ~~Report Xcode 16+ API compatibility issue to arkavo-org/idb repository~~ ✓
2. ~~Request rebuild with runtime API detection~~ ✓
3. Continue using IDB companion as fallback for Xcode 16+

**Update**: The Arkavo × FB-IDB team is actively working on a fix to support both API versions.