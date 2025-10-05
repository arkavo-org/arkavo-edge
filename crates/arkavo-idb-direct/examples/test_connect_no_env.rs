use arkavo_idb_direct::{IdbDirect, TargetType};

fn main() {
    println!("=== Test Connect Without DEVELOPER_DIR ===\n");

    // Show current environment
    println!("Current DEVELOPER_DIR: {:?}", std::env::var("DEVELOPER_DIR"));
    
    // Show xcode-select path
    let output = std::process::Command::new("xcode-select")
        .arg("-p")
        .output()
        .expect("Failed to run xcode-select");
    println!("xcode-select -p: {}", String::from_utf8_lossy(&output.stdout).trim());

    println!("\n1. Initializing IDB...");
    let mut idb = match IdbDirect::new() {
        Ok(idb) => {
            println!("   ✓ Initialized");
            println!("   Version: {}", IdbDirect::version());
            idb
        }
        Err(e) => {
            eprintln!("   ✗ Failed: {:?}", e);
            return;
        }
    };

    println!("\n2. Attempting connect...");
    let device_id = "F76602B2-EC91-4A32-BBAA-E36ADDBF83C4";
    
    match idb.connect_target(device_id, TargetType::Simulator) {
        Ok(()) => println!("   ✓ Connected!"),
        Err(e) => eprintln!("   ✗ Failed: {:?}", e),
    }

    println!("\n=== Test Complete ===");
}