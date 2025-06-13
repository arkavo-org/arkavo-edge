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

    if !idb_binary.exists() || !frameworks_archive.exists() {
        download_and_extract_idb(&idb_binary, &frameworks_archive);
    }

    // Tell Rust where to find the files for embedding
    println!(
        "cargo:rustc-env=IDB_COMPANION_PATH={}",
        idb_binary.display()
    );
    println!(
        "cargo:rustc-env=IDB_FRAMEWORKS_ARCHIVE={}",
        frameworks_archive.display()
    );
}

fn download_and_extract_idb(binary_path: &Path, frameworks_archive_path: &Path) {
    eprintln!("Downloading IDB companion and frameworks...");

    // Download the combined archive
    let download_url = "https://github.com/arkavo-org/idb/releases/download/1.2.0-arkavo.0/idb_companion-1.2.0-arkavo.0-macos-arm64.tar.gz";
    let temp_dir = env::temp_dir();
    let tar_path = temp_dir.join("idb-companion-combined.tar.gz");

    // Download using curl
    let status = Command::new("curl")
        .args(["-L", "-o", tar_path.to_str().unwrap(), download_url])
        .status()
        .expect("Failed to execute curl");

    if !status.success() {
        panic!("Failed to download idb_companion from {}", download_url);
    }

    // Extract the tarball
    let extract_dir = temp_dir.join("idb_extract");
    fs::create_dir_all(&extract_dir).expect("Failed to create extraction directory");

    let status = Command::new("tar")
        .args([
            "-xzf",
            tar_path.to_str().unwrap(),
            "-C",
            extract_dir.to_str().unwrap(),
        ])
        .status()
        .expect("Failed to execute tar");

    if !status.success() {
        panic!("Failed to extract idb_companion tarball");
    }

    // Copy the binary
    let src_binary = extract_dir.join("bin").join("idb_companion");
    if src_binary.exists() {
        fs::copy(&src_binary, binary_path).expect("Failed to copy idb_companion binary");
    } else {
        panic!(
            "idb_companion binary not found at expected location: {}",
            src_binary.display()
        );
    }

    // Make it executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(binary_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(binary_path, perms).unwrap();
    }

    // Package the frameworks
    let frameworks_dir = extract_dir.join("Frameworks");
    if frameworks_dir.exists() {
        eprintln!(
            "Found Frameworks directory at: {}",
            frameworks_dir.display()
        );

        // List framework contents for debugging
        if let Ok(entries) = fs::read_dir(&frameworks_dir) {
            eprintln!("Frameworks found:");
            for entry in entries.flatten() {
                eprintln!("  - {}", entry.file_name().to_string_lossy());
            }
        }

        // Create tar.gz archive of frameworks
        let status = Command::new("tar")
            .current_dir(&extract_dir)
            .args([
                "-czf",
                frameworks_archive_path.to_str().unwrap(),
                "Frameworks",
            ])
            .status()
            .expect("Failed to create frameworks archive");

        if !status.success() {
            panic!("Failed to create frameworks archive");
        }

        // Verify the archive was created and has content
        if let Ok(metadata) = fs::metadata(frameworks_archive_path) {
            eprintln!(
                "Successfully packaged IDB frameworks: {} bytes",
                metadata.len()
            );
            if metadata.len() == 0 {
                panic!("Frameworks archive is empty!");
            }
        } else {
            panic!(
                "Failed to create frameworks archive at: {}",
                frameworks_archive_path.display()
            );
        }
    } else {
        panic!("Frameworks directory not found in extracted archive");
    }

    // Clean up
    let _ = fs::remove_file(&tar_path);
    let _ = fs::remove_dir_all(&extract_dir);

    eprintln!("Successfully downloaded and extracted IDB companion");
}
