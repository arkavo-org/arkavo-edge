# Bug Report: Segmentation Fault in libidb_direct v1.3.2-arkavo.0 [FIXED in v1.3.2-arkavo.1]

## Summary
The libidb_direct library (v1.3.2-arkavo.0) crashes with a segmentation fault when calling `idb_connect_target()` on macOS with Xcode 16.2.

## Environment
- **OS**: macOS 15.5 (24F74)
- **Architecture**: Apple Silicon (arm64)
- **Xcode Version**: 16.2 (Build 16C5032a)
- **libidb_direct Version**: v1.3.2-arkavo.0
- **Simulator**: iPhone 16 Pro Max, iOS 18.2

## Bug Description
When attempting to connect to a simulator using `idb_connect_target()`, the application crashes with a segmentation fault. The crash occurs consistently across multiple test scenarios.

## Steps to Reproduce
1. Initialize IDB with `idb_initialize()` - succeeds
2. Call `idb_connect_target("F76602B2-EC91-4A32-BBAA-E36ADDBF83C4", IDB_TARGET_SIMULATOR)`
3. Application crashes immediately with SIGSEGV

## Expected Behavior
The function should successfully connect to the simulator without crashing.

## Actual Behavior
Segmentation fault occurs with the following characteristics:
- **Signal**: SIGSEGV (Segmentation fault: 11)
- **Exception Type**: EXC_BAD_ACCESS
- **Exception Subtype**: KERN_INVALID_ADDRESS at 0x0000000000000003
- **Crash Location**: `objc_msgSend` called from `__idb_connect_target_block_invoke_2`

## Crash Analysis
From the crash report:
```
Exception Type:  EXC_BAD_ACCESS (SIGSEGV)
Exception Subtype: KERN_INVALID_ADDRESS at 0x0000000000000003

Thread 0 Crashed:: main  Dispatch queue: com.arkavo.idb_adaptive_sync
0   libobjc.A.dylib                0x186e28e08 objc_msgSend + 8
1   test_connect_no_env            0x1040b3a54 __idb_connect_target_block_invoke_2 + 1344
2   libdispatch.dylib              0x186cc625c _dispatch_client_callout + 16
3   libdispatch.dylib              0x186ca87a8 _dispatch_lane_barrier_sync_invoke_and_complete + 56
4   test_connect_no_env            0x1040b349c idb_connect_target + 364
```

The crash suggests that an Objective-C message is being sent to an invalid object (address 0x3). This indicates either:
1. An uninitialized or NULL object pointer
2. Memory corruption
3. Incorrect type casting in the Objective-C bridge code

## Code to Reproduce
```rust
use arkavo_idb_direct::{IdbDirect, TargetType};

fn main() {
    let mut idb = IdbDirect::new().expect("Failed to initialize");
    println!("Initialized successfully");
    
    // This line causes the crash
    idb.connect_target("F76602B2-EC91-4A32-BBAA-E36ADDBF83C4", TargetType::Simulator)
        .expect("Failed to connect");
}
```

## Additional Information
- The crash occurs regardless of whether DEVELOPER_DIR is set
- The simulator is properly booted and visible via `xcrun simctl list devices booted`
- The library successfully loads CoreSimulator classes during initialization
- The same device ID works correctly with the IDB companion backend

## Suggested Investigation Areas
1. Check the `__idb_connect_target_block_invoke_2` implementation for proper object initialization
2. Verify all Objective-C objects are properly retained before use
3. Check for any API changes in CoreSimulator framework between Xcode versions
4. Review the dispatch queue usage in `com.arkavo.idb_adaptive_sync`

## Resolution
This issue has been fixed in libidb_direct v1.3.2-arkavo.1. The segmentation fault no longer occurs when connecting to simulators.

### Remaining Issues in v1.3.2-arkavo.1
While the crash is fixed, the following features still encounter errors:
- Tap functionality: "No compatible touch API found"
- Screenshot functionality: Returns OperationFailed

## Workaround (for v1.3.2-arkavo.0)
Using `IDB_BACKEND=companion` avoids the crash in the older version.

## Full Crash Report (v1.3.2-arkavo.0)
The complete crash report is available at:
`~/Library/Logs/DiagnosticReports/test_connect_no_env-2025-06-13-184608.ips`

Key registers at crash:
- x0: 0x0000000000000003 (invalid object pointer)
- x1: 0x0000000201e2f22a (selector: "integerValue")
- pc: 0x0000000186e28e08 (objc_msgSend + 8)