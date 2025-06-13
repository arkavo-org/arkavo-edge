use std::env;
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

    // Get the path to the vendor directory
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let vendor_path = PathBuf::from(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("vendor")
        .join("idb");

    let lib_path = vendor_path.join("libidb_direct.a");
    
    // Check if the static library exists
    if !lib_path.exists() {
        panic!(
            "libidb_direct.a not found at {:?}. Please run CI download step or manually download from https://github.com/arkavo-org/idb/releases/tag/1.3.0-arkavo.0",
            lib_path
        );
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
            panic!(
                "libidb_direct.a is not built for arm64 architecture. Found:\n{}",
                stdout
            );
        }
    }

    // Tell cargo to link the static library
    println!("cargo:rustc-link-search=native={}", vendor_path.display());
    println!("cargo:rustc-link-lib=static=idb_direct");

    // Link required macOS frameworks
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=IOKit");
        
        // CoreSimulator is a private framework in /Library/Developer/PrivateFrameworks
        let lib_dev_frameworks = PathBuf::from("/Library/Developer/PrivateFrameworks");
        if lib_dev_frameworks.join("CoreSimulator.framework").exists() {
            println!("cargo:rustc-link-search=framework={}", lib_dev_frameworks.display());
            println!("cargo:rustc-link-lib=framework=CoreSimulator");
        }
    }

    // Add the include path for headers
    println!("cargo:include={}/include", vendor_path.display());
    
    // Generate bindings
    let bindings = bindgen::Builder::default()
        .header(vendor_path.join("include/idb_direct.h").to_str().unwrap())
        .header(vendor_path.join("include/idb_direct_shm.h").to_str().unwrap())
        .clang_arg(format!("-I{}/include", vendor_path.display()))
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
    println!("cargo:rerun-if-changed={}/include/idb_direct.h", vendor_path.display());
}