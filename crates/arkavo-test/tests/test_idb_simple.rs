use arkavo_test::mcp::idb_wrapper::IdbWrapper;

#[test]
fn test_idb_init_sync() {
    println!("Testing IDB initialization...");
    
    // Check if embedded bytes are available
    #[cfg(target_os = "macos")]
    {
        println!("IDB_COMPANION_PATH env at compile time: {:?}", env!("IDB_COMPANION_PATH"));
        println!("Checking embedded binary size...");
        // Note: We can't directly access the static bytes from here, but we can check through initialization
    }
    
    match IdbWrapper::initialize() {
        Ok(_) => println!("IDB initialized successfully"),
        Err(e) => println!("IDB initialization failed: {}", e),
    }
    
    // Try to get binary path
    match IdbWrapper::get_binary_path() {
        Ok(path) => {
            println!("Binary path: {}", path.display());
            println!("Binary exists: {}", path.exists());
            
            if path.exists() {
                let metadata = std::fs::metadata(&path).unwrap();
                println!("Binary size: {} bytes", metadata.len());
            }
        },
        Err(e) => println!("Failed to get binary path: {}", e),
    }
}