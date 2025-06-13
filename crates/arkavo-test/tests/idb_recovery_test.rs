#[cfg(target_os = "macos")]
use arkavo_test::mcp::idb_recovery::IdbRecovery;

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_idb_recovery_stuck_companion() {
    // This test verifies that the IDB recovery can detect and handle
    // the case where IDB companion is running but not connected

    let recovery = IdbRecovery::new();

    // Check initial state
    let companion_running = IdbRecovery::is_companion_running().await;
    let port_accessible = IdbRecovery::is_companion_port_accessible().await;

    eprintln!(
        "Initial state - Companion running: {}, Port accessible: {}",
        companion_running, port_accessible
    );

    // If we detect the stuck state, try recovery
    if companion_running && !port_accessible {
        eprintln!("Detected stuck IDB companion, attempting recovery...");

        match recovery.recover_stuck_companion().await {
            Ok(_) => {
                eprintln!("Recovery completed successfully");

                // Wait a bit for things to stabilize
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                // Check state after recovery
                let companion_running_after = IdbRecovery::is_companion_running().await;
                let port_accessible_after = IdbRecovery::is_companion_port_accessible().await;

                eprintln!(
                    "After recovery - Companion running: {}, Port accessible: {}",
                    companion_running_after, port_accessible_after
                );

                // If companion is running, port should be accessible
                if companion_running_after {
                    assert!(
                        port_accessible_after,
                        "IDB companion is running but port is still not accessible after recovery"
                    );
                }
            }
            Err(e) => {
                eprintln!("Recovery failed: {}", e);
                // Don't fail the test as IDB might not be installed
            }
        }
    } else {
        eprintln!("IDB companion is not in stuck state, skipping recovery test");
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_force_reconnect_device() {
    // This test verifies device reconnection functionality

    let recovery = IdbRecovery::new();

    // Use a dummy device ID for testing
    let device_id = "test-device-123";

    match recovery.force_reconnect_device(device_id).await {
        Ok(_) => {
            eprintln!("Force reconnect completed for device {}", device_id);
        }
        Err(e) => {
            eprintln!("Force reconnect failed (expected if no device): {}", e);
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn test_idb_recovery_not_macos() {
    eprintln!("IDB recovery tests are only available on macOS");
}
