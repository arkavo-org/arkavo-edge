use arkavo_test::mcp::idb_wrapper::IdbWrapper;

fn main() {
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
}