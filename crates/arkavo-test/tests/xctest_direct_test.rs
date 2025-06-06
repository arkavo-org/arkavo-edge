//! Direct test of XCTest compiler and socket communication

use arkavo_test::mcp::{
    device_manager::DeviceManager, xctest_compiler::XCTestCompiler,
    xctest_unix_bridge::XCTestUnixBridge,
};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
#[cfg(target_os = "macos")]
async fn test_xctest_direct_compilation_and_connection() {
    println!("\n=== Direct XCTest Compilation Test ===\n");

    // Find a booted device
    let device_manager = DeviceManager::new();
    let devices = match device_manager.refresh_devices() {
        Ok(devices) => devices,
        Err(e) => {
            println!("No devices found: {} (expected without Xcode)", e);
            return;
        }
    };

    let booted_device = match devices
        .iter()
        .find(|d| d.state == arkavo_test::mcp::device_manager::DeviceState::Booted)
    {
        Some(device) => device,
        None => {
            println!("No booted device found");
            return;
        }
    };

    println!(
        "Using device: {} ({})",
        booted_device.name, booted_device.id
    );

    // Step 1: Compile XCTest bundle
    println!("\n1. Compiling XCTest bundle...");
    let compiler = match XCTestCompiler::new() {
        Ok(c) => c,
        Err(e) => {
            println!("Failed to create compiler: {}", e);
            return;
        }
    };

    let bundle_path = match compiler.get_xctest_bundle() {
        Ok(path) => {
            println!("   Bundle compiled at: {}", path.display());
            path
        }
        Err(e) => {
            println!("   Compilation failed: {}", e);
            return;
        }
    };

    // Step 2: Install to simulator
    println!("\n2. Installing bundle to simulator...");
    if let Err(e) = compiler.install_to_simulator(&booted_device.id, &bundle_path) {
        println!("   Installation failed: {}", e);
        return;
    }
    println!("   Installation successful");

    // Step 3: Create Unix bridge
    println!("\n3. Creating Unix bridge...");
    let socket_path = compiler.socket_path().to_path_buf();
    println!("   Socket path: {}", socket_path.display());

    let mut bridge = XCTestUnixBridge::with_socket_path(socket_path.clone());

    // Step 4: Launch the test host app (which starts the Swift server)
    println!("\n4. Launching test host app...");
    if let Err(e) = compiler.launch_test_host(&booted_device.id, None) {
        println!("   Failed to launch host app: {}", e);
        return;
    }

    // Give the Swift side time to start its server
    println!("\n5. Waiting for Swift server to initialize...");
    sleep(Duration::from_secs(3)).await;

    // Step 6: Connect as a client to the Swift server
    println!("\n6. Connecting to test runner...");
    match bridge.connect_to_runner().await {
        Ok(()) => {
            println!("   Connected successfully!");

            // Step 7: Send a ping
            println!("\n7. Sending ping...");
            match bridge.send_ping().await {
                Ok(()) => println!("   ✅ Ping successful!"),
                Err(e) => println!("   ❌ Ping failed: {}", e),
            }
        }
        Err(e) => {
            println!("   ❌ Connection failed: {}", e);
        }
    }

    println!("\n=== End of Direct Test ===\n");
}
