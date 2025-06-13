#![cfg(target_os = "macos")]

use arkavo_test::mcp::idb_wrapper::IdbWrapper;
use std::process::Command;

#[tokio::test]
async fn test_calibration_tap_sequence() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("\n=== CALIBRATION TAP SEQUENCE TEST ===\n");

    // Step 1: Get booted device
    eprintln!("1. Getting booted device...");
    let device_output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .output()?;

    if !device_output.status.success() {
        eprintln!("   No booted devices found");
        return Ok(());
    }

    let device_str = String::from_utf8_lossy(&device_output.stdout);

    // Extract device ID
    let device_id = if let Some(line) = device_str.lines().find(|line| line.contains("(Booted)")) {
        if let Some(start) = line.rfind('(') {
            if let Some(end) = line[start..].find(')') {
                let udid_part = &line[start + 1..start + end];
                // Extract just the UDID part
                if let Some(space_pos) = udid_part.find(' ') {
                    &udid_part[..space_pos]
                } else {
                    udid_part
                }
            } else {
                eprintln!("   Could not extract device ID");
                return Ok(());
            }
        } else {
            eprintln!("   Could not extract device ID");
            return Ok(());
        }
    } else {
        eprintln!("   No booted device found in output");
        return Ok(());
    };

    eprintln!("   Found device: {}", device_id);

    // Step 2: Check if ArkavoReference app is available
    eprintln!("\n2. Checking for ArkavoReference app...");
    let app_check = Command::new("xcrun")
        .args([
            "simctl",
            "get_app_container",
            device_id,
            "com.arkavo.reference",
        ])
        .output()?;

    if !app_check.status.success() {
        eprintln!("   ArkavoReference app not installed, skipping test");
        return Ok(());
    }
    eprintln!("   ArkavoReference app is available");

    // Step 3: Launch the app
    eprintln!("\n3. Launching ArkavoReference app...");
    let launch_output = Command::new("xcrun")
        .args(["simctl", "launch", device_id, "com.arkavo.reference"])
        .output()?;

    if !launch_output.status.success() {
        eprintln!(
            "   Failed to launch app: {}",
            String::from_utf8_lossy(&launch_output.stderr)
        );
        return Ok(());
    }
    eprintln!("   App launched successfully");

    // Wait for app to start
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Step 4: Test the calibration tap sequence
    eprintln!("\n4. Testing calibration tap sequence...");

    // Use the same percentage-based positions as the calibration system
    let screen_width = 393.0; // iPhone 16 logical width
    let screen_height = 852.0; // iPhone 16 logical height

    let test_points = [
        (screen_width * 0.2, screen_height * 0.2), // 20%, 20%
        (screen_width * 0.8, screen_height * 0.2), // 80%, 20%
        (screen_width * 0.5, screen_height * 0.5), // 50%, 50%
        (screen_width * 0.2, screen_height * 0.8), // 20%, 80%
        (screen_width * 0.8, screen_height * 0.8), // 80%, 80%
    ];

    for (idx, (x, y)) in test_points.iter().enumerate() {
        eprintln!("   Tapping point {}: ({:.0}, {:.0})", idx + 1, x, y);

        match IdbWrapper::tap(device_id, *x, *y).await {
            Ok(result) => {
                eprintln!("   ✓ Tap {} succeeded: {:?}", idx + 1, result);
            }
            Err(e) => {
                eprintln!("   ✗ Tap {} failed: {}", idx + 1, e);
            }
        }

        // Small delay between taps
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Step 5: Check for calibration results
    eprintln!("\n5. Checking for calibration results...");
    let docs_output = Command::new("xcrun")
        .args([
            "simctl",
            "get_app_container",
            device_id,
            "com.arkavo.reference",
            "data",
        ])
        .output()?;

    if docs_output.status.success() {
        let container_path = String::from_utf8_lossy(&docs_output.stdout)
            .trim()
            .to_string();
        let calibration_file = format!("{}/Documents/calibration_results.json", container_path);

        if std::path::Path::new(&calibration_file).exists() {
            eprintln!("   ✓ Found calibration results!");
            let contents = std::fs::read_to_string(&calibration_file)?;
            eprintln!("   Results: {}", contents);
        } else {
            eprintln!("   ✗ No calibration results found at: {}", calibration_file);
        }
    }

    eprintln!("\n=== TEST COMPLETE ===\n");

    Ok(())
}
