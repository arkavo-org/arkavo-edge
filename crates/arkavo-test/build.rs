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

    println!("cargo:rerun-if-changed=build.rs");

    // Determine architecture
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let arch_suffix = match target_arch.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => {
            eprintln!(
                "Warning: Unsupported architecture {} for idb_companion",
                target_arch
            );
            return;
        }
    };

    let out_dir = env::var("OUT_DIR").unwrap();
    let idb_binary_path = Path::new(&out_dir).join(format!("idb_companion_{}", arch_suffix));

    // Check if we already have the binary
    if !idb_binary_path.exists() || is_placeholder(&idb_binary_path) {
        download_and_extract_idb_companion(&idb_binary_path);
    }

    // Tell Rust where to find the binary for embedding
    println!(
        "cargo:rustc-env=IDB_COMPANION_PATH={}",
        idb_binary_path.display()
    );
}

fn is_placeholder(path: &Path) -> bool {
    if let Ok(content) = fs::read(path) {
        // Check if it's our placeholder script
        content.len() < 1000 || content.starts_with(b"#!/bin/bash")
    } else {
        true
    }
}

fn download_and_extract_idb_companion(target_path: &Path) {
    eprintln!("Downloading idb_companion from GitHub releases...");
    
    // Download the universal tarball
    let download_url = "https://github.com/facebook/idb/releases/download/v1.1.8/idb-companion.universal.tar.gz";
    let temp_dir = env::temp_dir();
    let tar_path = temp_dir.join("idb-companion.universal.tar.gz");
    
    // Download the tarball
    let status = Command::new("curl")
        .args(&["-L", "-o", tar_path.to_str().unwrap(), download_url])
        .status()
        .expect("Failed to execute curl");
    
    if !status.success() {
        panic!("Failed to download idb_companion from {}", download_url);
    }
    
    eprintln!("Extracting idb_companion...");
    
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
    
    // Find the idb_companion binary in the extracted files
    let idb_binary = extract_dir.join("idb-companion.universal").join("bin").join("idb_companion");
    if !idb_binary.exists() {
        // Try alternate location
        let alt_binary = extract_dir.join("bin").join("idb_companion");
        if alt_binary.exists() {
            fs::copy(&alt_binary, target_path).expect("Failed to copy idb_companion binary");
        } else {
            panic!("idb_companion binary not found in extracted tarball at expected locations");
        }
    } else {
        // Copy to target location
        fs::copy(&idb_binary, target_path).expect("Failed to copy idb_companion binary");
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
    
    eprintln!("Successfully downloaded and prepared idb_companion for embedding");
}
