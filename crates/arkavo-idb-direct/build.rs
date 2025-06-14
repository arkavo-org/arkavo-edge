use std::{env, fs, io};
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Architecture check
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    if target_arch != "aarch64" {
        eprintln!("Error: IDB Direct FFI only supports arm64/aarch64 architecture.");
        eprintln!("Current architecture: {}", target_arch);
        eprintln!("Please build on an Apple Silicon Mac or use --target aarch64-apple-darwin");
        panic!("Unsupported architecture");
    }

    // Get paths for storing dependencies
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Store in target directory (parent of OUT_DIR) to keep them with other build artifacts
    // This follows Rust's convention of putting build artifacts in target/
    let deps_path = out_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("idb_deps");

    let lib_path = deps_path.join("libidb_direct.a");
    let include_dir = deps_path.join("include");

    // If previous download failed and left incomplete or corrupted files, clean them up
    if lib_path.exists() {
        // Check if the file is valid
        #[cfg(target_os = "macos")]
        {
            let file_check = Command::new("file")
                .args([lib_path.to_str().unwrap()])
                .output()
                .expect("Failed to run 'file' command");

            let file_type = String::from_utf8_lossy(&file_check.stdout);

            // If not a valid object file, remove it
            if file_type.contains("ASCII text") || file_type.contains("HTML") || !file_type.contains("ar archive") {
                eprintln!("Found invalid library file, removing: {}", file_type);
                let _ = fs::remove_file(&lib_path);
            }
        }
    }

    // Clean up incomplete include directory if it exists but headers are missing
    if include_dir.exists() {
        let has_headers = include_dir.join("idb_direct.h").exists() && 
                       include_dir.join("idb_direct_shm.h").exists();
        if !has_headers {
            eprintln!("Found incomplete include directory, removing");
            let _ = fs::remove_dir_all(&include_dir);
        }
    }

    // Get current version
    let expected_version = "1.4.0-arkavo";
    let version_file = deps_path.join("version.txt");
    let current_version = if version_file.exists() {
        fs::read_to_string(&version_file).unwrap_or_default()
    } else {
        String::new()
    };

    // Check if we need to update (library missing, headers missing, or version mismatch)
    if !lib_path.exists() || !include_dir.exists() || current_version.trim() != expected_version {
        // Ensure dependencies directory exists
        fs::create_dir_all(&deps_path).expect("Failed to create dependencies directory");

        // Attempt to download the library archive
        eprintln!("libidb_direct.a not found. Downloading from GitHub release...");

        // URL of the tar.gz archive
        let archive_url = "https://github.com/arkavo-org/idb/releases/download/1.4.0-arkavo/libidb_direct-1.4.0-arkavo-macos-arm64.tar.gz";

        // Download to target directory instead of temp dir
        let download_dir = deps_path.join("download");
        fs::create_dir_all(&download_dir).expect("Failed to create download directory");
        let archive_path = download_dir.join("libidb_direct.tar.gz");

        // Download archive using reqwest
        download_file(archive_url, &archive_path).expect("Failed to download archive file");

        // Extract directly to the deps_path
        // First, create a temporary extract directory within the deps_path
        let extract_dir = deps_path.join("extract");
        fs::create_dir_all(&extract_dir).expect("Failed to create extraction directory");

        // Clear any previous extraction
        if extract_dir.exists() {
            let _ = fs::remove_dir_all(&extract_dir);
            fs::create_dir_all(&extract_dir).expect("Failed to recreate extraction directory");
        }

        // First verify the archive is valid by listing its contents
        let verify_output = Command::new("tar")
            .args([
                "-tzf",
                archive_path.to_str().unwrap()
            ])
            .output()
            .expect("Failed to verify tar archive");

        if !verify_output.status.success() {
            let stderr = String::from_utf8_lossy(&verify_output.stderr);
            panic!("Invalid tar archive: {}", stderr);
        }

        // Check contents to make sure it has what we need
        let contents = String::from_utf8_lossy(&verify_output.stdout);
        eprintln!("Archive contents:\n{}", contents);

        // Check for expected files
        if !contents.contains("libidb_direct.a") || !contents.contains("include/") {
            panic!("Archive doesn't contain expected files (libidb_direct.a and/or include/ directory)");
        }

        // Extract the archive with --strip-components=1 to avoid nested directory
        let extract_output = Command::new("tar")
            .args([
                "-xzf",
                archive_path.to_str().unwrap(),
                "-C",
                extract_dir.to_str().unwrap(),
                "--strip-components=1"  // Skip the top-level directory
            ])
            .output()
            .expect("Failed to execute tar command");

        if !extract_output.status.success() {
            let stderr = String::from_utf8_lossy(&extract_output.stderr);
            eprintln!("Warning: Failed to extract with --strip-components=1: {}", stderr);

            // Fall back to regular extraction
            let fallback_output = Command::new("tar")
                .args([
                    "-xzf",
                    archive_path.to_str().unwrap(),
                    "-C",
                    extract_dir.to_str().unwrap()
                ])
                .output()
                .expect("Failed to execute tar command");

            if !fallback_output.status.success() {
                let stderr = String::from_utf8_lossy(&fallback_output.stderr);
                panic!("Failed to extract the archive: {}", stderr);
            }

            // Check if we need to handle a nested directory
            let entries = fs::read_dir(&extract_dir).expect("Failed to read extract directory");
            let mut dir_count = 0;
            let mut found_dir = None;

            for entry in entries.filter_map(Result::ok) {
                if entry.path().is_dir() {
                    dir_count += 1;
                    found_dir = Some(entry.path());
                }
            }

            // If exactly one directory was found, it's likely our nested dir
            if dir_count == 1 && found_dir.is_some() {
                let nested_dir = found_dir.unwrap();
                eprintln!("Found single directory in extraction: {}", nested_dir.display());

                // Check if it contains our files
                let has_lib = nested_dir.join("libidb_direct.a").exists();
                let has_include = nested_dir.join("include").exists();

                if has_lib && has_include {
                    eprintln!("Found library and include in nested directory, using its contents");

                    // Move files from nested directory to extract_dir
                    for entry in fs::read_dir(&nested_dir).expect("Failed to read nested dir") {
                        if let Ok(entry) = entry {
                            let src_path = entry.path();
                            let file_name = entry.file_name();
                            let dst_path = extract_dir.join(&file_name);

                            if src_path.is_dir() {
                                // Copy directory
                                copy_dir_contents(&src_path, &dst_path)
                                    .expect("Failed to copy nested directory");
                            } else {
                                // Copy file
                                fs::copy(&src_path, &dst_path)
                                    .expect("Failed to copy nested file");
                            }

                            eprintln!("  Moved {} to {}", src_path.display(), dst_path.display());
                        }
                    }

                    // Clean up the nested directory
                    let _ = fs::remove_dir_all(&nested_dir);
                }
            }
        }

        // Debug: List extracted contents
        eprintln!("Extracted contents:");
        if let Ok(entries) = fs::read_dir(&extract_dir) {
            for entry in entries.flatten() {
                eprintln!("  - {}", entry.file_name().to_string_lossy());
            }
        }

        // Move files from extract_dir to final locations
        let src_lib = extract_dir.join("libidb_direct.a");
        if src_lib.exists() {
            // If the lib_path already exists but is different, replace it
            if lib_path.exists() {
                fs::remove_file(&lib_path).expect("Failed to remove existing library");
            }
            fs::rename(&src_lib, &lib_path).expect("Failed to move library file");
            eprintln!("Moved library file: {} -> {}", src_lib.display(), lib_path.display());
        } else {
            panic!("Library file not found at: {}", src_lib.display());
        }

        // Move the include directory
        let src_include = extract_dir.join("include");
        if src_include.exists() {
            // If include_dir already exists, remove it first
            if include_dir.exists() {
                fs::remove_dir_all(&include_dir).expect("Failed to remove existing include directory");
            }
            fs::rename(&src_include, &include_dir).expect("Failed to move include directory");
            eprintln!("Moved include directory: {} -> {}", src_include.display(), include_dir.display());
        } else {
            panic!("Include directory not found at: {}", src_include.display());
        }

        // Clean up the extraction directory since we've moved everything we need
        let _ = fs::remove_dir_all(&extract_dir);
        eprintln!("Cleaned up extraction directory: {}", extract_dir.display());

        // Keep the archive for caching
        eprintln!("Archive kept for caching: {}", archive_path.display());

        // Verify that the library was extracted and valid
        if !lib_path.exists() {
            panic!(
                "Library file not found after extraction. Expected at: {}",
                lib_path.display()
            );
        }

        // Verify that include directory exists with required headers
        let idb_h = include_dir.join("idb_direct.h");
        let idb_shm_h = include_dir.join("idb_direct_shm.h");

        if !idb_h.exists() || !idb_shm_h.exists() {
            panic!("Required header files missing after extraction");
        }

        // Verify the library is a valid object file
        #[cfg(target_os = "macos")]
        {
            let file_check = Command::new("file")
                .args([lib_path.to_str().unwrap()])
                .output()
                .expect("Failed to run 'file' command");

            let file_type = String::from_utf8_lossy(&file_check.stdout);

            // Ensure it's actually a library file
            if !file_type.contains("ar archive") && !file_type.contains("current ar archive") {
                panic!("Extracted file is not a valid library: {}", file_type);
            }

            eprintln!("Library file type: {}", file_type.trim());
        }

        // Write version file
        let expected_version = "1.4.0-arkavo";
        let version_file = deps_path.join("version.txt");
        fs::write(&version_file, expected_version)
            .expect("Failed to write version file");

        eprintln!("Successfully downloaded and extracted library files (version {})", expected_version);
    }

    // Verify architecture is arm64 on macOS
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("otool")
            .args(["-h", lib_path.to_str().unwrap()])
            .output()
            .expect("Failed to run otool");

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Check for ARM64 architecture (cputype 16777228 = 0x100000C = ARM64)
        if !stdout.contains("16777228") && !stdout.contains("ARM64") && !stdout.contains("arm64") {
            // Check if the file is actually a valid object file
            let file_check = Command::new("file")
                .args([lib_path.to_str().unwrap()])
                .output()
                .expect("Failed to run 'file' command");

            let file_type = String::from_utf8_lossy(&file_check.stdout);

            panic!(
                "libidb_direct.a is not built for arm64 architecture or is not a valid library file.\nFile info: {}\notool output:\n{}",
                file_type, stdout
            );
        }
    }

    // Tell cargo to link the static library
    println!("cargo:rustc-link-search=native={}", deps_path.display());
    println!("cargo:rustc-link-lib=static=idb_direct");

    // Link required macOS frameworks
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=IOKit");

        // CoreSimulator is a private framework in /Library/Developer/PrivateFrameworks
        let lib_dev_frameworks = PathBuf::from("/Library/Developer/PrivateFrameworks");
        if lib_dev_frameworks.join("CoreSimulator.framework").exists() {
            println!(
                "cargo:rustc-link-search=framework={}",
                lib_dev_frameworks.display()
            );
            println!("cargo:rustc-link-lib=framework=CoreSimulator");
        }
    }

    // Add the include path for headers
    let include_dir = deps_path.join("include");
    println!("cargo:include={}", include_dir.display());

    // Generate bindings
    let bindings = bindgen::Builder::default()
        .header(include_dir.join("idb_direct.h").to_str().unwrap())
        .header(include_dir.join("idb_direct_shm.h").to_str().unwrap())
        .clang_arg(format!("-I{}", include_dir.display()))
        // Whitelist the types and functions we need
        .allowlist_type("idb_.*")
        .allowlist_function("idb_.*")
        .allowlist_var("IDB_.*")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    // Rerun if the library or headers change
    println!("cargo:rerun-if-changed={}", lib_path.display());
    println!(
        "cargo:rerun-if-changed={}/idb_direct.h",
        include_dir.display()
    );
    println!(
        "cargo:rerun-if-changed={}/idb_direct_shm.h",
        include_dir.display()
    );
}

// Function to download a file using reqwest
fn download_file(url: &str, destination: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Downloading {} to {}", url, destination.display());

    // Remove any existing file at the destination
    if destination.exists() {
        eprintln!("Removing existing file at {}", destination.display());
        fs::remove_file(destination)?;
    }

    // Create a blocking reqwest client
    let client = reqwest::blocking::Client::builder()
        .user_agent("arkavo-idb-direct-build/1.0")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    // Fetch the file
    let response = client.get(url).send()?;

    // Check if the request was successful
    if !response.status().is_success() {
        return Err(format!("Failed to download file: HTTP {} - {}", 
                          response.status(), 
                          response.text().unwrap_or_default()).into());
    }

    // Check content type if available
    if let Some(content_type) = response.headers().get("content-type") {
        let content_type_str = content_type.to_str().unwrap_or("");
        eprintln!("Content-Type: {}", content_type_str);

        // Warn if we're not getting binary data
        if content_type_str.starts_with("text/") || content_type_str.contains("html") {
            eprintln!("WARNING: Received text content type: {}", content_type_str);
        }
    }

    // Get the response bytes
    let bytes = response.bytes()?;

    // Check if we have reasonable size (tar.gz should be at least a few KB)
    if bytes.len() < 1000 {
        return Err(format!("Downloaded file too small: {} bytes", bytes.len()).into());
    }

    // Check if it looks like a tar.gz (gzip magic number is 0x1f, 0x8b)
    if bytes.len() >= 2 && (bytes[0] != 0x1f || bytes[1] != 0x8b) {
        // Show a preview of what we got
        let preview = String::from_utf8_lossy(&bytes[..std::cmp::min(100, bytes.len())]);
        eprintln!("WARNING: File doesn't look like gzip. First bytes: {}", preview);
    }

    // Write the bytes to the destination file
    fs::write(destination, &bytes)?;

    eprintln!("Download complete: {} bytes", bytes.len());
    Ok(())
}

// Helper function to recursively copy directory contents
fn copy_dir_contents(src: &PathBuf, dst: &PathBuf) -> io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_contents(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;

            // Set executable permissions if it's a binary on Unix systems
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if src_path.extension().map_or(false, |ext| ext == "a" || ext == "dylib" || ext == "so")
                    || src_path.file_name().map_or(false, |name| !name.to_string_lossy().contains(".")) {
                    if let Ok(metadata) = fs::metadata(&src_path) {
                        let mode = metadata.permissions().mode();
                        if mode & 0o111 != 0 {  // If original is executable
                            let mut perms = fs::metadata(&dst_path)?.permissions();
                            perms.set_mode(0o755);  // rwxr-xr-x
                            fs::set_permissions(&dst_path, perms)?;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}