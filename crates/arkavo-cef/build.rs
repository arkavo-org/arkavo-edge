use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Skip CEF build for non-macOS platforms initially
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("darwin") {
        println!("cargo:warning=CEF is currently only supported on macOS");
        return;
    }

    println!("cargo:rerun-if-changed=cef-bridge/");
    println!("cargo:rerun-if-changed=../../vendor/cef");

    // Get the workspace root (where Cargo.toml is)
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let cef_root = PathBuf::from(&manifest_dir).join("../../vendor/cef");

    // Normalize the path
    let cef_root = cef_root.canonicalize().unwrap_or_else(|_| {
        // If canonicalize fails, try to construct absolute path manually
        std::fs::canonicalize(&manifest_dir)
            .unwrap_or_else(|_| PathBuf::from(&manifest_dir))
            .join("../../vendor/cef")
    });

    if !cef_root.exists() {
        eprintln!(
            "\n============================================================================="
        );
        eprintln!(
            "CEF not found at {:?}",
            cef_root.canonicalize().unwrap_or(cef_root.clone())
        );
        eprintln!("\nTo download and setup CEF, run:");
        eprintln!("    ./scripts/setup-cef.sh");
        eprintln!("\nOr download manually from:");
        eprintln!("    https://cef-builds.spotifycdn.com/index.html");
        eprintln!(
            "=============================================================================\n"
        );

        // Don't fail the build - allow compilation without CEF
        println!("cargo:warning=CEF not found - skipping CEF bridge build");
        return;
    }

    // Check if CEF DLL wrapper is built
    let wrapper_lib = cef_root.join("build_wrapper/libcef_dll_wrapper/libcef_dll_wrapper.a");
    if !wrapper_lib.exists() {
        eprintln!(
            "\n============================================================================="
        );
        eprintln!("CEF DLL wrapper not built");
        eprintln!("\nRunning setup script to build wrapper...");
        eprintln!(
            "=============================================================================\n"
        );

        let status = Command::new("bash")
            .arg("../../scripts/setup-cef.sh")
            .status();

        match status {
            Ok(s) if s.success() => {
                println!("cargo:warning=CEF DLL wrapper built successfully");
            }
            _ => {
                println!("cargo:warning=Failed to build CEF DLL wrapper - skipping");
                return;
            }
        }
    }

    // Build our CEF bridge
    let bridge_dir = PathBuf::from("cef-bridge");

    // Configure with CMake - let cmake crate handle build directory
    let mut config = cmake::Config::new(&bridge_dir);
    config
        .define("CEF_ROOT", cef_root.to_str().unwrap())
        .define("CMAKE_BUILD_TYPE", "Release");

    println!("cargo:warning=Building CEF bridge...");
    let dst = config.build();

    // Link the CEF libraries
    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // Link CEF framework
    println!("cargo:rustc-link-lib=framework=CEF");
    println!(
        "cargo:rustc-link-search=framework={}/Release",
        cef_root.display()
    );

    // Link system frameworks
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");

    // Link C++ standard library
    println!("cargo:rustc-link-lib=c++");

    println!("cargo:warning=CEF bridge built successfully");
}
