#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use arkavo_test::mcp::device_manager::DeviceManager;
    use arkavo_test::mcp::test_target_app::TestTargetApp;
    use arkavo_test::mcp::xctest_compiler::XCTestCompiler;
    use arkavo_test::mcp::xctest_unix_bridge::XCTestUnixBridge;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_xctest_with_target_app() -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("\n=== XCTest With Target App Test ===\n");

        // Get a booted device
        let device_manager = DeviceManager::new();
        let devices = device_manager.get_all_devices();

        let booted_device = devices
            .iter()
            .find(|d| d.state == arkavo_test::mcp::device_manager::DeviceState::Booted)
            .ok_or("No booted device found")?;

        eprintln!(
            "Using device: {} ({})",
            booted_device.name, booted_device.id
        );

        // Step 1: Build and install test target app
        eprintln!("\n1. Installing test target app...");
        let target_app = TestTargetApp::new()?;
        target_app.build_and_install(&booted_device.id)?;
        eprintln!(
            "✅ Test target app installed: {}",
            target_app.app_bundle_id()
        );

        // Step 2: Setup XCTest with target app
        eprintln!("\n2. Setting up XCTest...");
        let compiler = XCTestCompiler::new()?;

        // Compile and install test bundle
        let bundle_path = compiler.get_xctest_bundle()?;
        eprintln!("✅ XCTest bundle compiled");

        compiler.install_to_simulator(&booted_device.id, &bundle_path)?;
        eprintln!("✅ XCTest bundle installed");

        // Start the Unix socket bridge
        let socket_path = compiler.socket_path().to_path_buf();
        let mut bridge = XCTestUnixBridge::with_socket_path(socket_path.clone());
        bridge.start().await?;
        eprintln!("✅ Unix socket bridge started");

        // Launch test host with target app
        eprintln!("\n3. Launching test host with target app...");
        compiler.launch_test_host(&booted_device.id, Some(&target_app.app_bundle_id()))?;

        // Give the host more time to initialize when target app is specified
        eprintln!("   Waiting for test host to initialize...");
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Wait for connection
        eprintln!("\n4. Waiting for connection...");
        match timeout(Duration::from_secs(10), bridge.wait_for_connection()).await {
            Ok(Ok(())) => {
                eprintln!("✅ XCTest runner connected!");

                // Try a simple command using coordinates (since text-based taps are not supported in bridge mode)
                eprintln!("\n5. Testing UI interaction with coordinate tap...");
                let tap_command = XCTestUnixBridge::create_coordinate_tap(100.0, 100.0);

                match timeout(Duration::from_secs(5), bridge.send_tap_command(tap_command)).await {
                    Ok(Ok(response)) => {
                        eprintln!("✅ Tap command succeeded: {:?}", response);
                    }
                    Ok(Err(e)) => {
                        eprintln!("❌ Tap command failed: {}", e);
                    }
                    Err(_) => {
                        eprintln!("❌ Tap command timed out");
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("❌ Connection failed: {}", e);
                return Err(e.into());
            }
            Err(_) => {
                eprintln!("❌ Connection timed out");
                return Err("Connection timeout".into());
            }
        }

        eprintln!("\n=== Test Complete ===");
        Ok(())
    }
}
