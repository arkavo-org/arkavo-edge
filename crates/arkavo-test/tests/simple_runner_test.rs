#[cfg(test)]
mod tests {
    use arkavo_test::mcp::xctest_simple_runner::XCTestSimpleRunner;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_simple_runner() -> Result<(), Box<dyn std::error::Error>> {
        // Create simple runner
        let runner = XCTestSimpleRunner::new()?;
        
        eprintln!("Socket path: {}", runner.socket_path().display());
        
        // Get booted device
        let device_manager = arkavo_test::mcp::device_manager::DeviceManager::new();
        let devices = device_manager.get_all_devices();
            
        let booted_device = devices
            .iter()
            .find(|d| d.state == arkavo_test::mcp::device_manager::DeviceState::Booted)
            .ok_or("No booted device found")?;
            
        eprintln!("Using device: {} ({})", booted_device.name, booted_device.id);
        
        // Run the simple server
        runner.run_on_simulator(&booted_device.id)?;
        
        // Give it time to start
        sleep(Duration::from_secs(2)).await;
        
        // Try to connect to the socket
        use std::os::unix::net::UnixStream;
        match UnixStream::connect(runner.socket_path()) {
            Ok(mut stream) => {
                eprintln!("Connected to socket!");
                
                // Send a test message
                use std::io::Write;
                stream.write_all(b"Hello from Rust")?;
                stream.flush()?;
                
                // Read response
                use std::io::Read;
                let mut buffer = [0u8; 1024];
                let n = stream.read(&mut buffer)?;
                let response = String::from_utf8_lossy(&buffer[..n]);
                eprintln!("Received response: {}", response);
                
                assert!(response.contains("Echo:"));
            }
            Err(e) => {
                eprintln!("Failed to connect to socket: {}", e);
                return Err(e.into());
            }
        }
        
        Ok(())
    }
}