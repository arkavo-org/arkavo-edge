#![cfg(target_os = "macos")]

use arkavo_test::mcp::idb_wrapper::IdbWrapper;

#[tokio::test]
async fn test_idb_tap_simple() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Testing IDB Tap Functionality ===\n");

    // Initialize IDB
    println!("1. Initializing IDB...");
    IdbWrapper::initialize()?;

    // First, let's list available targets to see what devices are available
    println!("2. Listing IDB targets...");
    match IdbWrapper::list_targets().await {
        Ok(targets) => {
            println!(
                "   IDB targets: {}",
                serde_json::to_string_pretty(&targets)?
            );
        }
        Err(e) => {
            println!("   ✗ Failed to list targets: {}", e);
            return Err(e.into());
        }
    }

    // Test device - let's use a different device ID that might be available
    let device_id = "325F1C75-3912-426F-9A7F-C533911A56E5";

    // Ensure companion is running
    println!(
        "\n3. Ensuring IDB companion is running for device {}...",
        device_id
    );
    match IdbWrapper::ensure_companion_running(device_id).await {
        Ok(_) => println!("   ✓ IDB companion is ready"),
        Err(e) => {
            println!("   ✗ Failed to ensure companion: {}", e);

            // Let's check what simulators are available
            println!("\n   Checking available simulators via simctl...");
            let simctl_output = std::process::Command::new("xcrun")
                .args(["simctl", "list", "devices", "booted"])
                .output()?;

            let output_str = String::from_utf8_lossy(&simctl_output.stdout);
            println!("   Booted simulators:\n{}", output_str);

            return Err(e.into());
        }
    }

    // Test tap at center of screen
    println!("\n4. Testing tap at (200, 400)...");
    match IdbWrapper::tap(device_id, 200.0, 400.0).await {
        Ok(result) => {
            println!("   ✓ Tap successful!");
            println!("   Result: {}", serde_json::to_string_pretty(&result)?);
        }
        Err(e) => {
            println!("   ✗ Tap failed: {}", e);
            return Err(e.into());
        }
    }

    // Test another tap
    println!("\n5. Testing tap at (100, 100)...");
    match IdbWrapper::tap(device_id, 100.0, 100.0).await {
        Ok(result) => {
            println!("   ✓ Second tap successful!");
            println!("   Result: {}", serde_json::to_string_pretty(&result)?);
        }
        Err(e) => {
            println!("   ✗ Second tap failed: {}", e);
            return Err(e.into());
        }
    }

    println!("\n✅ IDB tap tests completed successfully!");

    Ok(())
}
