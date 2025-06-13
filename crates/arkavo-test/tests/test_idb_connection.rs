#[cfg(test)]
mod test_idb_connection {
    use arkavo_test::mcp::idb_wrapper::IdbWrapper;
    use arkavo_test::mcp::simulator_manager::SimulatorManager;

    #[tokio::test]
    async fn test_idb_connection_to_simulator() {
        eprintln!("=== Testing IDB Connection to Simulator ===");

        // Initialize IDB
        match IdbWrapper::initialize() {
            Ok(_) => eprintln!("✓ IDB initialized successfully"),
            Err(e) => {
                eprintln!("✗ IDB initialization failed: {}", e);
                return;
            }
        }

        // Get a booted simulator
        let sim_manager = SimulatorManager::new();
        let booted_devices = sim_manager.get_booted_devices();

        if booted_devices.is_empty() {
            eprintln!("✗ No booted simulators found. Please boot a simulator first.");
            return;
        }

        let device = booted_devices[0];
        eprintln!(
            "✓ Found booted simulator: {} ({})",
            device.name, device.udid
        );

        // Test list-targets command
        eprintln!("\n--- Testing list-targets command ---");
        match IdbWrapper::list_targets().await {
            Ok(targets) => {
                eprintln!("✓ list-targets succeeded");
                eprintln!(
                    "Targets: {}",
                    serde_json::to_string_pretty(&targets).unwrap_or_default()
                );

                // Check if our device is in the list
                if let Some(arr) = targets.as_array() {
                    let device_found = arr
                        .iter()
                        .any(|t| t.get("udid").and_then(|u| u.as_str()) == Some(&device.udid));

                    if device_found {
                        eprintln!("✓ Target device {} found in IDB targets", device.udid);
                    } else {
                        eprintln!("✗ Target device {} NOT found in IDB targets", device.udid);
                    }
                }
            }
            Err(e) => {
                eprintln!("✗ list-targets failed: {}", e);
            }
        }

        // Test a simple tap to verify connection works
        eprintln!("\n--- Testing tap command ---");
        let test_x = 100.0;
        let test_y = 100.0;

        match IdbWrapper::tap(&device.udid, test_x, test_y).await {
            Ok(result) => {
                eprintln!("✓ Tap command succeeded");
                eprintln!(
                    "Result: {}",
                    serde_json::to_string_pretty(&result).unwrap_or_default()
                );
            }
            Err(e) => {
                eprintln!("✗ Tap command failed: {}", e);
                eprintln!("This might indicate IDB is not properly connected to the simulator");
            }
        }

        eprintln!("\n=== Test Complete ===");
    }
}
