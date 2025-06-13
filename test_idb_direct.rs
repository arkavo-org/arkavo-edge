use arkavo_test::mcp::idb_wrapper::IdbWrapper;

#[tokio::main]
async fn main() {
    println!("Testing IDB initialization and execution...");
    
    // Initialize IDB
    match IdbWrapper::initialize() {
        Ok(_) => println!("IDB initialized successfully"),
        Err(e) => {
            println!("Failed to initialize IDB: {}", e);
            return;
        }
    }
    
    // Try to list targets
    println!("\nAttempting to list targets...");
    match IdbWrapper::list_targets().await {
        Ok(targets) => {
            println!("Successfully listed targets: {}", 
                serde_json::to_string_pretty(&targets).unwrap_or_default());
        }
        Err(e) => {
            println!("Failed to list targets: {}", e);
        }
    }
}