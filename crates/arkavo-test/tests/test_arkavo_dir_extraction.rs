use arkavo_test::mcp::idb_wrapper::IdbWrapper;
use arkavo_test::Result;
use std::fs;
use std::path::PathBuf;

#[tokio::test]
async fn test_idb_extracts_to_arkavo_directory() -> Result<()> {
    println!("🔍 Testing IDB extraction to .arkavo directory...");
    
    // Get current working directory
    let cwd = std::env::current_dir()?;
    println!("Current working directory: {}", cwd.display());
    
    // Clean up any existing .arkavo directory for clean test
    let arkavo_dir = cwd.join(".arkavo");
    if arkavo_dir.exists() {
        println!("Cleaning up existing .arkavo directory...");
        fs::remove_dir_all(&arkavo_dir)?;
    }
    
    // Initialize IDB wrapper
    println!("\n📦 Initializing IDB wrapper...");
    IdbWrapper::initialize()?;
    
    // Check that .arkavo directory was created
    assert!(arkavo_dir.exists(), ".arkavo directory should exist");
    println!("✅ .arkavo directory created at: {}", arkavo_dir.display());
    
    // Check for IDB subdirectory
    let idb_dir = arkavo_dir.join("idb");
    assert!(idb_dir.exists(), ".arkavo/idb directory should exist");
    println!("✅ IDB directory created at: {}", idb_dir.display());
    
    // Check for binary
    let binary_path = idb_dir.join("bin").join("idb_companion");
    assert!(binary_path.exists(), "IDB companion binary should exist");
    println!("✅ IDB companion binary extracted to: {}", binary_path.display());
    
    // Check binary is executable
    let metadata = fs::metadata(&binary_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        assert!(mode & 0o111 != 0, "Binary should be executable");
        println!("✅ Binary has executable permissions: {:o}", mode);
    }
    
    // Check for frameworks
    let frameworks_dir = idb_dir.join("Frameworks");
    if frameworks_dir.exists() {
        println!("✅ Frameworks directory found at: {}", frameworks_dir.display());
        
        // List some framework contents
        if let Ok(entries) = fs::read_dir(&frameworks_dir) {
            println!("\nFrameworks found:");
            for entry in entries.take(5) {
                if let Ok(entry) = entry {
                    println!("  - {}", entry.file_name().to_string_lossy());
                }
            }
        }
    }
    
    // Check if .gitignore exists and contains .arkavo
    let gitignore_path = cwd.join(".gitignore");
    if gitignore_path.exists() {
        let content = fs::read_to_string(&gitignore_path)?;
        if content.contains(".arkavo") {
            println!("\n✅ .gitignore already contains .arkavo");
        } else {
            println!("\n⚠️  .gitignore exists but doesn't contain .arkavo");
            println!("   Consider adding '.arkavo/' to your .gitignore");
        }
    } else {
        println!("\n⚠️  No .gitignore file found");
        println!("   Consider creating one and adding '.arkavo/'");
    }
    
    // Test that IDB can list targets (verifies it's working)
    println!("\n🧪 Testing IDB functionality...");
    let targets = IdbWrapper::list_targets().await?;
    println!("✅ IDB is functional, found devices/simulators");
    
    println!("\n🎉 All tests passed! IDB is properly extracted to .arkavo directory");
    
    Ok(())
}