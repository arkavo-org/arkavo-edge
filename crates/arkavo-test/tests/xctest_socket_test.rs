#[cfg(test)]
mod tests {
    use arkavo_test::mcp::device_manager::DeviceManager;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_xctest_socket_connection() -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("\n=== XCTest Socket Connection Test ===\n");
        
        // Get a booted device
        let device_manager = DeviceManager::new();
        let devices = device_manager.get_all_devices();
        
        let booted_device = devices
            .iter()
            .find(|d| d.state == arkavo_test::mcp::device_manager::DeviceState::Booted)
            .ok_or("No booted device found")?;
            
        eprintln!("Using device: {} ({})", booted_device.name, booted_device.id);
        
        // Setup XCTest
        use std::sync::Arc;
        let setup_kit = arkavo_test::mcp::xctest_setup_tool::XCTestSetupKit::new(Arc::new(device_manager));
        
        eprintln!("Setting up XCTest...");
        use arkavo_test::mcp::server::Tool;
        
        let params = serde_json::json!({
            "device_id": booted_device.id,
            "force_reinstall": false
        });
        
        match timeout(Duration::from_secs(30), setup_kit.execute(params)).await {
            Ok(Ok(result)) => {
                eprintln!("✅ XCTest setup succeeded!");
                eprintln!("Result: {}", serde_json::to_string_pretty(&result)?);
            }
            Ok(Err(e)) => {
                eprintln!("❌ XCTest setup failed: {}", e);
                return Err(e.into());
            }
            Err(_) => {
                eprintln!("❌ XCTest setup timed out!");
                return Err("Setup timed out".into());
            }
        }
        
        Ok(())
    }
}