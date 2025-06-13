#[tokio::test]
async fn test_idb_initialization() {
    use arkavo_test::mcp::idb_wrapper::IdbWrapper;

    println!("Testing IDB initialization...");

    match IdbWrapper::initialize() {
        Ok(_) => println!("IDB initialized successfully"),
        Err(e) => println!("IDB initialization failed: {}", e),
    }

    // Try to get binary path
    match IdbWrapper::get_binary_path() {
        Ok(path) => println!("Binary path: {}", path.display()),
        Err(e) => println!("Failed to get binary path: {}", e),
    }

    // Try list_targets
    match IdbWrapper::list_targets().await {
        Ok(targets) => println!("List targets result: {:?}", targets),
        Err(e) => println!("List targets failed: {}", e),
    }
}
