use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();

    // Compile appropriate implementation based on platform
    match target_os.as_str() {
        "macos" | "ios" => {
            // Use real implementation on Apple platforms
            cc::Build::new()
                .file("src/bridge/ios_impl.c")
                .warnings(true)
                .compile("ios_bridge");

            // Link with CoreFoundation framework
            println!("cargo:rustc-link-lib=framework=CoreFoundation");

            // Setup idb_companion embedding for macOS
            if target_os == "macos" {
                setup_idb_companion();
            }
        }
        _ => {
            // Use stub on other platforms
            cc::Build::new()
                .file("src/bridge/ios_stub.c")
                .warnings(false)
                .compile("ios_bridge");
        }
    }
}

fn setup_idb_companion() {
    // idb_companion embedding setup
    // This embeds Meta's idb_companion tool (MIT licensed) for iOS simulator automation
    // See THIRD-PARTY-LICENSES.md for full license information
    
    eprintln!("[build.rs] Setting up IDB companion...");

    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir);
    
    // Check if we need to download IDB and frameworks
    let idb_binary = out_path.join("idb_companion");
    let frameworks_archive = out_path.join("frameworks.tar.gz");
    
    if !idb_binary.exists() {
        download_idb_companion(&idb_binary);
    }
    
    if !frameworks_archive.exists() {
        package_frameworks(&frameworks_archive);
    }

    // Tell Rust where to find the files for embedding
    println!("cargo:rustc-env=IDB_COMPANION_PATH={}", idb_binary.display());
    println!("cargo:rustc-env=IDB_FRAMEWORKS_ARCHIVE={}", frameworks_archive.display());
}

fn download_idb_companion(target_path: &Path) {
    eprintln!("Downloading IDB companion...");
    
    // Download the universal tarball
    let download_url = "https://github.com/facebook/idb/releases/download/v1.1.8/idb-companion.universal.tar.gz";
    let temp_dir = env::temp_dir();
    let tar_path = temp_dir.join("idb-companion.universal.tar.gz");
    
    // Download using curl
    let status = Command::new("curl")
        .args(&["-L", "-o", tar_path.to_str().unwrap(), download_url])
        .status()
        .expect("Failed to execute curl");
    
    if !status.success() {
        panic!("Failed to download idb_companion from {}", download_url);
    }
    
    // Extract the tarball
    let extract_dir = temp_dir.join("idb_extract");
    fs::create_dir_all(&extract_dir).expect("Failed to create extraction directory");
    
    let status = Command::new("tar")
        .args(&["-xzf", tar_path.to_str().unwrap(), "-C", extract_dir.to_str().unwrap()])
        .status()
        .expect("Failed to execute tar");
    
    if !status.success() {
        panic!("Failed to extract idb_companion tarball");
    }
    
    // Find and copy the binary
    let locations = [
        extract_dir.join("idb-companion.universal").join("bin").join("idb_companion"),
        extract_dir.join("bin").join("idb_companion"),
    ];
    
    let mut found = false;
    for location in &locations {
        if location.exists() {
            fs::copy(location, target_path).expect("Failed to copy idb_companion binary");
            found = true;
            break;
        }
    }
    
    if !found {
        panic!("idb_companion binary not found in expected locations");
    }
    
    // Make it executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(target_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(target_path, perms).unwrap();
    }
    
    // Clean up
    let _ = fs::remove_file(&tar_path);
    let _ = fs::remove_dir_all(&extract_dir);
    
    eprintln!("Successfully downloaded idb_companion");
}

fn package_frameworks(archive_path: &Path) {
    eprintln!("Packaging IDB frameworks...");
    
    // Check common locations for IDB frameworks
    let framework_locations = [
        "/opt/homebrew/Frameworks",      // Apple Silicon homebrew
        "/usr/local/Frameworks",         // Intel Mac homebrew
        "/Library/Frameworks",           // System location
    ];
    
    let mut frameworks_found = false;
    let temp_dir = env::temp_dir().join("idb_frameworks_staging");
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("Failed to create staging directory");
    
    let frameworks_dir = temp_dir.join("Frameworks");
    fs::create_dir_all(&frameworks_dir).expect("Failed to create Frameworks directory");
    
    // Required frameworks
    let required_frameworks = [
        "FBControlCore.framework",
        "FBDeviceControl.framework", 
        "FBSimulatorControl.framework",
        "IDBCompanionUtilities.framework",
        "XCTestBootstrap.framework",
        "IDBGRPCSwift.framework"
    ];
    
    // Find and copy frameworks
    for location in &framework_locations {
        let location_path = Path::new(location);
        if location_path.exists() {
            let mut found_count = 0;
            for framework in &required_frameworks {
                let framework_path = location_path.join(framework);
                if framework_path.exists() {
                    eprintln!("Found {} in {}", framework, location);
                    let dest = frameworks_dir.join(framework);
                    
                    // Use cp -RL to dereference symlinks and preserve framework structure
                    let status = Command::new("cp")
                        .args(&["-RL", framework_path.to_str().unwrap(), dest.to_str().unwrap()])
                        .status()
                        .expect("Failed to copy framework");
                        
                    if status.success() {
                        found_count += 1;
                    }
                }
            }
            
            if found_count > 0 {
                frameworks_found = true;
                break;
            }
        }
    }
    
    if frameworks_found {
        // Create tar.gz archive
        let status = Command::new("tar")
            .current_dir(&temp_dir)
            .args(&["-czf", archive_path.to_str().unwrap(), "Frameworks"])
            .status()
            .expect("Failed to create frameworks archive");
            
        if !status.success() {
            panic!("Failed to create frameworks archive");
        }
        
        eprintln!("Successfully packaged IDB frameworks");
    } else {
        eprintln!("Warning: IDB frameworks not found on system");
        eprintln!("IDB companion will require system-installed frameworks at runtime");
        eprintln!("Install via: brew install facebook/fb/idb-companion");
        
        // Create empty archive so build doesn't fail
        fs::write(archive_path, b"").expect("Failed to create placeholder archive");
    }
    
    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);
}

