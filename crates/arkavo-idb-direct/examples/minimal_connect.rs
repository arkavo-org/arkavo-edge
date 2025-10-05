use arkavo_idb_direct::{IdbDirect, TargetType};

fn main() {
    println!("=== Minimal Connect Test ===\n");

    // Initialize
    println!("1. Initializing IDB...");
    let mut idb = match IdbDirect::new() {
        Ok(idb) => {
            println!("   ✓ Initialized");
            idb
        }
        Err(e) => {
            eprintln!("   ✗ Failed: {:?}", e);
            return;
        }
    };

    // Try connect with debug output
    println!("\n2. Attempting connect...");
    let device_id = "F76602B2-EC91-4A32-BBAA-E36ADDBF83C4";
    
    println!("   Device ID: {}", device_id);
    println!("   Target type: Simulator");
    println!("   About to call connect_target()...");
    
    match idb.connect_target(device_id, TargetType::Simulator) {
        Ok(()) => println!("   ✓ Connected!"),
        Err(e) => eprintln!("   ✗ Failed: {:?}", e),
    }

    println!("\n=== Test Complete ===");
}