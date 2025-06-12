use std::process::Command;

fn main() {
    println!("Testing IDB companion with critical environment variables...\n");
    
    let binary_path = "./target/arkavo_idb/bin/idb_companion";
    
    // Test 1: Without environment variables (should fail with SIGKILL)
    println!("Test 1: Running WITHOUT environment variables:");
    let result = Command::new(binary_path)
        .args(["--udid", "test-device", "--only", "simulator"])
        .output();
        
    match result {
        Ok(output) => {
            println!("Exit status: {:?}", output.status);
            if let Some(code) = output.status.code() {
                println!("Exit code: {}", code);
                if code == 9 {
                    println!("❌ SIGKILL detected as expected without env vars");
                }
            }
        }
        Err(e) => println!("Failed to run: {}", e),
    }
    
    println!("\n---\n");
    
    // Test 2: With critical environment variables (should work)
    println!("Test 2: Running WITH critical environment variables:");
    let mut cmd = Command::new(binary_path);
    cmd.args(["--udid", "test-device", "--only", "simulator"])
        .env("DYLD_DISABLE_LIBRARY_VALIDATION", "1")
        .env("DYLD_FORCE_FLAT_NAMESPACE", "1")
        .env("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES");
        
    println!("Environment set:");
    println!("  DYLD_DISABLE_LIBRARY_VALIDATION=1");
    println!("  DYLD_FORCE_FLAT_NAMESPACE=1");
    println!("  OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES");
    
    // Use spawn to see if it stays running
    match cmd.spawn() {
        Ok(mut child) => {
            println!("✓ Process spawned successfully!");
            
            // Wait briefly to see if it gets killed
            std::thread::sleep(std::time::Duration::from_secs(2));
            
            match child.try_wait() {
                Ok(Some(status)) => {
                    println!("Process exited with status: {:?}", status);
                    if let Some(code) = status.code() {
                        if code == 9 {
                            println!("❌ Still got SIGKILL even with env vars");
                        }
                    }
                }
                Ok(None) => {
                    println!("✅ Process is still running! Environment variables fixed the issue.");
                    // Kill it cleanly
                    let _ = child.kill();
                }
                Err(e) => println!("Error checking status: {}", e),
            }
        }
        Err(e) => println!("Failed to spawn: {}", e),
    }
}