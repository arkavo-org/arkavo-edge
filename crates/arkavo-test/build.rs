use std::env;
use std::fs;
use std::path::Path;

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
    if !idb_binary_path.exists() {
        create_idb_companion_placeholder(&idb_binary_path, arch_suffix);
    }

    // Tell Rust where to find the binary for embedding
    println!(
        "cargo:rustc-env=IDB_COMPANION_PATH={}",
        idb_binary_path.display()
    );

    // Ensure the file exists (even if it's just a placeholder)
    if !idb_binary_path.exists() {
        // Create a minimal placeholder to satisfy include_bytes!
        fs::write(&idb_binary_path, b"placeholder").expect("Failed to create placeholder");
    }
}

fn create_idb_companion_placeholder(target_path: &Path, arch: &str) {
    eprintln!("Setting up idb_companion for {}...", arch);

    // Try to find installed idb_companion
    let possible_sources = vec![
        "/opt/homebrew/bin/idb_companion", // Homebrew on Apple Silicon
        "/usr/local/bin/idb_companion",    // Homebrew on Intel
    ];

    let mut source_found = None;
    for source in &possible_sources {
        if Path::new(source).exists() {
            source_found = Some(source);
            break;
        }
    }

    if let Some(source) = source_found {
        // Copy the actual binary
        eprintln!("Copying idb_companion from: {}", source);
        if let Err(e) = fs::copy(source, target_path) {
            eprintln!("Warning: Failed to copy idb_companion: {}", e);
            create_placeholder(target_path, arch);
        } else {
            eprintln!("Successfully copied idb_companion for embedding");
            // Make it executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(target_path).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(target_path, perms).unwrap();
            }
        }
    } else {
        eprintln!("idb_companion not found, creating placeholder");
        create_placeholder(target_path, arch);
    }
}

fn create_placeholder(target_path: &Path, arch: &str) {
    let placeholder_script = r#"#!/bin/bash
# idb_companion placeholder
# Install with: brew install idb-companion
echo "idb_companion placeholder - not installed"
echo "Architecture: {arch}"
exit 1
"#;

    let script_content = placeholder_script.replace("{arch}", arch);
    fs::write(target_path, script_content).expect("Failed to write placeholder");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(target_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(target_path, perms).unwrap();
    }
}
