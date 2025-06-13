use arkavo_idb_direct::IdbDirect;

fn main() {
    println!("=== Simple IDB Direct FFI Test ===\n");
    
    println!("1. Getting version (static call)...");
    let version = IdbDirect::version();
    println!("   Version: {}", version);
    
    println!("\n2. Attempting initialization...");
    match IdbDirect::new() {
        Ok(idb) => {
            println!("   ✓ IDB initialized successfully!");
            
            println!("\n3. Performing safety check...");
            match idb.safety_check() {
                Ok(()) => println!("   ✓ Simulator available"),
                Err(e) => println!("   ⚠ Safety check failed: {:?}", e),
            }
            
            println!("\n4. Cleanup...");
            drop(idb);
            println!("   ✓ IDB shutdown");
        }
        Err(e) => {
            eprintln!("   ✗ Failed to initialize: {:?}", e);
            eprintln!("\nThis might be due to:");
            eprintln!("- Missing CoreSimulator runtime");
            eprintln!("- Static library linking issues");
            eprintln!("- Objective-C runtime initialization");
        }
    }
    
    println!("\n=== Test Complete ===");
}