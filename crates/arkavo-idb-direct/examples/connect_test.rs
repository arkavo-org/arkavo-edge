use arkavo_idb_direct::{IdbDirect, TargetType};

fn main() {
    println!("=== IDB Direct FFI Connect Test ===\n");
    
    println!("1. Setting DEVELOPER_DIR environment...");
    std::env::set_var("DEVELOPER_DIR", "/Applications/Xcode-beta.app/Contents/Developer");
    
    println!("2. Initializing IDB Direct FFI...");
    let mut idb = match IdbDirect::new() {
        Ok(idb) => {
            println!("   ✓ IDB initialized successfully");
            println!("   Version: {}", IdbDirect::version());
            idb
        }
        Err(e) => {
            eprintln!("   ✗ Failed to initialize: {:?}", e);
            return;
        }
    };
    
    println!("\n3. Attempting to connect to simulator...");
    let device_id = "4A05B20A-349D-4EC5-B796-8F384798268B";
    println!("   Device ID: {}", device_id);
    
    match idb.connect_target(device_id, TargetType::Simulator) {
        Ok(()) => println!("   ✓ Connected successfully!"),
        Err(e) => {
            eprintln!("   ✗ Failed to connect: {:?}", e);
            eprintln!("\nThis might indicate:");
            eprintln!("- The Xcode 16+ API fix is not yet in v1.3.2");
            eprintln!("- Or there's another issue with CoreSimulator");
        }
    }
    
    println!("\n=== Test Complete ===");
}