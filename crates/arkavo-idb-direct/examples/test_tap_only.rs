use arkavo_idb_direct::{IdbDirect, TargetType};

fn main() {
    println!("=== Test Tap Only ===\n");

    let mut idb = IdbDirect::new().expect("Failed to initialize");
    println!("✓ Initialized (v{})", IdbDirect::version());

    let device_id = "F76602B2-EC91-4A32-BBAA-E36ADDBF83C4";
    idb.connect_target(device_id, TargetType::Simulator)
        .expect("Failed to connect");
    println!("✓ Connected");

    // Test tap
    println!("\nTesting tap at (195, 422)...");
    match idb.tap(195.0, 422.0) {
        Ok(()) => println!("✓ Tap successful!"),
        Err(e) => eprintln!("✗ Tap failed: {:?}", e),
    }

    println!("\n=== Complete ===");
}