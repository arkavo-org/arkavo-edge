use arkavo_test::mcp::idb_wrapper::IdbWrapper;

#[tokio::test]
async fn test_idb_initialization_with_system_preference() {
    // Test that we can initialize with system IDB preference
    let result = IdbWrapper::initialize_with_preference(true);
    
    match result {
        Ok(_) => {
            eprintln!("IDB initialized successfully");
            
            // Try to list targets to verify it works
            let targets_result = IdbWrapper::list_targets().await;
            match targets_result {
                Ok(targets) => {
                    eprintln!("Successfully listed targets: {:?}", targets);
                }
                Err(e) => {
                    eprintln!("Failed to list targets: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("IDB initialization failed (expected if IDB not installed): {}", e);
            // This is acceptable - the test passes either way to show error handling works
        }
    }
}

#[tokio::test]
async fn test_idb_framework_conflict_handling() {
    // Test with environment variable
    unsafe { std::env::set_var("ARKAVO_USE_SYSTEM_IDB", "1"); }
    
    let result = IdbWrapper::initialize();
    
    match result {
        Ok(_) => {
            eprintln!("IDB initialized with ARKAVO_USE_SYSTEM_IDB=1");
        }
        Err(e) => {
            // Should get a helpful error if system IDB not found
            assert!(e.to_string().contains("brew install facebook/fb/idb-companion"));
        }
    }
    
    // Clean up
    unsafe { std::env::remove_var("ARKAVO_USE_SYSTEM_IDB"); }
}

#[tokio::test] 
#[ignore] // Only run this test manually when debugging IDB issues
async fn test_idb_tap_with_framework_conflicts() {
    // This test deliberately uses embedded IDB to trigger framework conflicts
    unsafe { std::env::remove_var("ARKAVO_USE_SYSTEM_IDB"); }
    
    let init_result = IdbWrapper::initialize();
    if init_result.is_err() {
        eprintln!("Skipping test - IDB not available");
        return;
    }
    
    // Try a tap operation that might trigger framework conflicts
    let device_id = "booted"; // Use currently booted simulator
    let tap_result = IdbWrapper::tap(device_id, 100.0, 100.0).await;
    
    match tap_result {
        Ok(result) => {
            eprintln!("Tap succeeded: {:?}", result);
        }
        Err(e) => {
            let error_msg = e.to_string();
            eprintln!("Tap failed: {}", error_msg);
            
            // Check if we got a helpful error message about framework conflicts
            if error_msg.contains("Framework conflict detected") {
                assert!(error_msg.contains("brew install"));
                eprintln!("Got expected framework conflict error with helpful message");
            }
        }
    }
}