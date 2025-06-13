# Testing IDB Direct FFI on Xcode 16

## Prerequisites

1. Ensure Xcode 16 is installed
2. Check your Xcode path:
   ```bash
   xcode-select -p
   ```
   If it's not pointing to Xcode 16, set it:
   ```bash
   sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
   ```

## Step 1: Build the Project

```bash
# Clean build to ensure fresh start
cargo clean

# Build with release mode
cargo build --release
```

## Step 2: Test Basic Functionality

```bash
# Test initialization and version
cargo run --example simple_init
```

Expected output:
```
=== IDB Direct FFI Example ===
Initializing IDB Direct FFI...
✓ IDB initialized successfully
Version: 1.3.2-arkavo.0
Shutting down...
✓ IDB shut down successfully
```

## Step 3: Boot a Simulator

```bash
# List available simulators
xcrun simctl list devices available | grep -A 10 "iPhone"

# Boot a simulator (replace with your device ID)
export DEVICE_ID="YOUR-DEVICE-ID"
xcrun simctl boot $DEVICE_ID

# Wait for it to fully boot
open -a Simulator
```

## Step 4: Test Connection (Expected to Work on Xcode 16)

```bash
# Test connection
cargo run --example connect_test
```

On Xcode 16, this should successfully connect. Output:
```
=== IDB Direct FFI Connect Test ===
1. Setting DEVELOPER_DIR environment...
2. Initializing IDB Direct FFI...
   ✓ IDB initialized successfully
   Version: 1.3.2-arkavo.0
3. Attempting to connect to simulator...
   Device ID: YOUR-DEVICE-ID
   ✓ Connected successfully!
```

## Step 5: Test Tap Functionality

```bash
# Test tap (requires connected simulator)
cargo run --example tap_test
```

Expected output:
```
=== IDB Direct FFI Tap Test ===
✓ Connected to simulator
Performing tap at (100, 100)...
✓ Tap successful!
```

## Step 6: Compare Performance

```bash
# Force Direct FFI backend
IDB_BACKEND=direct cargo run --example tap_test

# Force Companion fallback
IDB_BACKEND=companion cargo run --example tap_test
```

The Direct FFI should show microsecond latency vs milliseconds for Companion.

## Debugging

If connection hangs:
```bash
# Run debug connect with timeout
cargo run --example debug_connect
```

This will show if the connection is hanging (indicates Xcode compatibility issue).

## Expected Results on Xcode 16

Since v1.3.2-arkavo.0 was built with Xcode 16+ compatibility in mind, it should:

1. ✅ Successfully initialize
2. ✅ Connect to simulator without hanging
3. ✅ Perform taps with microsecond latency
4. ✅ Fall back gracefully if issues occur

## Verifying the Integration

The static library should be at:
```bash
ls -la vendor/idb/
# Should show libidb_direct.a (~19KB)
```

Check which backend is being used:
```bash
# The examples will print which backend is active
```

## CI Verification

If testing in CI, ensure:
1. The `IDB_VERSION` and `IDB_SHA256` in `.github/workflows/feature.yaml` match
2. The static library is properly cached
3. Architecture is arm64/aarch64