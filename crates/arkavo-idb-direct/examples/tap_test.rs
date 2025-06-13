use arkavo_idb_direct::{IdbDirect, TargetType};
use std::time::Instant;

fn main() {
    println!("=== IDB Direct FFI Tap Test ===\n");
    
    // Use the iPhone 16 Pro Max simulator
    let device_id = "4A05B20A-349D-4EC5-B796-8F384798268B";
    
    println!("1. Initializing IDB Direct FFI...");
    let mut idb = match IdbDirect::new() {
        Ok(idb) => {
            println!("   ✓ IDB initialized successfully");
            println!("   Version: {}", IdbDirect::version());
            idb
        }
        Err(e) => {
            eprintln!("   ✗ Failed to initialize IDB: {:?}", e);
            return;
        }
    };
    
    // Perform safety check
    println!("\n2. Performing safety check...");
    match idb.safety_check() {
        Ok(()) => println!("   ✓ Simulator is available"),
        Err(e) => {
            eprintln!("   ✗ Safety check failed: {:?}", e);
            eprintln!("   Make sure a simulator is booted");
            return;
        }
    }
    
    println!("\n3. Connecting to device: {}", device_id);
    match idb.connect_target(device_id, TargetType::Simulator) {
        Ok(()) => println!("   ✓ Connected successfully"),
        Err(e) => {
            eprintln!("   ✗ Failed to connect: {:?}", e);
            return;
        }
    }
    
    // Take a screenshot before tap
    println!("\n4. Taking screenshot before tap...");
    let _before_screenshot = match idb.take_screenshot() {
        Ok(screenshot) => {
            println!("   ✓ Screenshot captured: {}x{} ({})", 
                screenshot.width, screenshot.height, screenshot.format);
            println!("   Size: {} bytes", screenshot.data().len());
            screenshot
        }
        Err(e) => {
            eprintln!("   ✗ Screenshot failed: {:?}", e);
            return;
        }
    };
    
    // Perform tap tests
    println!("\n5. Performing tap tests...");
    
    // Center of screen tap
    let center_x = 195.0;
    let center_y = 422.0;
    
    println!("\n   Test 1: Center tap at ({}, {})", center_x, center_y);
    let start = Instant::now();
    match idb.tap(center_x, center_y) {
        Ok(()) => {
            let latency = start.elapsed();
            println!("   ✓ Tap successful - Latency: {:?}", latency);
            println!("   Microseconds: {}μs", latency.as_micros());
        }
        Err(e) => eprintln!("   ✗ Tap failed: {:?}", e),
    }
    
    // Wait a bit
    std::thread::sleep(std::time::Duration::from_millis(500));
    
    // Multiple taps to test performance
    println!("\n   Test 2: Performance test - 10 rapid taps");
    let mut total_latency = std::time::Duration::ZERO;
    let mut min_latency = std::time::Duration::MAX;
    let mut max_latency = std::time::Duration::ZERO;
    
    for i in 0..10 {
        let x = 100.0 + (i as f64 * 20.0);
        let y = 200.0;
        
        let start = Instant::now();
        match idb.tap(x, y) {
            Ok(()) => {
                let latency = start.elapsed();
                total_latency += latency;
                min_latency = min_latency.min(latency);
                max_latency = max_latency.max(latency);
            }
            Err(e) => eprintln!("   ✗ Tap {} failed: {:?}", i + 1, e),
        }
        
        // Small delay between taps
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    
    println!("   ✓ Performance results:");
    println!("     Average latency: {:?}", total_latency / 10);
    println!("     Min latency: {:?}", min_latency);
    println!("     Max latency: {:?}", max_latency);
    println!("     Average μs: {}μs", (total_latency / 10).as_micros());
    
    // Take screenshot after taps
    println!("\n6. Taking screenshot after taps...");
    let _after_screenshot = match idb.take_screenshot() {
        Ok(screenshot) => {
            println!("   ✓ Screenshot captured: {}x{}", screenshot.width, screenshot.height);
            screenshot
        }
        Err(e) => {
            eprintln!("   ✗ Screenshot failed: {:?}", e);
            return;
        }
    };
    
    // Disconnect
    println!("\n7. Disconnecting...");
    match idb.disconnect_target() {
        Ok(()) => println!("   ✓ Disconnected successfully"),
        Err(e) => eprintln!("   ✗ Disconnect failed: {:?}", e),
    }
    
    println!("\n=== Test Complete ===");
}