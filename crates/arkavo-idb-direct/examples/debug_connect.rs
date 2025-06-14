use arkavo_idb_direct::{IdbDirect, TargetType};
use std::time::{Duration, Instant};

fn main() {
    println!("=== IDB Direct FFI Debug Connect ===\n");

    // Set environment
    std::env::set_var(
        "DEVELOPER_DIR",
        "/Applications/Xcode-beta.app/Contents/Developer",
    );

    println!("1. Initializing IDB...");
    let mut idb = IdbDirect::new().expect("Failed to initialize");
    println!("   ✓ Initialized");

    println!("\n2. Starting connect with timeout...");
    let device_id = "4A05B20A-349D-4EC5-B796-8F384798268B";

    // Spawn connect in a thread so we can timeout
    let start = Instant::now();
    let handle = std::thread::spawn(move || {
        println!("   Calling connect_target...");
        match idb.connect_target(device_id, TargetType::Simulator) {
            Ok(()) => println!("   ✓ Connected!"),
            Err(e) => eprintln!("   ✗ Error: {:?}", e),
        }
    });

    // Wait up to 5 seconds
    std::thread::sleep(Duration::from_secs(5));

    if handle.is_finished() {
        println!("   Connect completed in {:?}", start.elapsed());
    } else {
        println!("   ✗ Connect timed out after 5 seconds");
        println!("   This suggests the library is hanging in connect_target");
        println!("   Likely due to blocking CoreSimulator API calls");
    }

    println!("\n=== Analysis ===");
    println!("The v1.3.2 library appears to have the API compatibility code");
    println!("but may still have issues with Xcode 26 beta");
}
