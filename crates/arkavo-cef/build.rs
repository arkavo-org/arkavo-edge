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

    // Check if CEF exists and has the required framework binary
    let framework_binary =
        cef_root.join("Release/Chromium Embedded Framework.framework/Chromium Embedded Framework");

    if !cef_root.exists() {
        panic!(
            "\n=============================================================================\n\
             CEF not found at {}\n\
             \n\
             To download and setup CEF, run:\n\
             ./scripts/setup-cef.sh\n\
             \n\
             Or download manually from:\n\
             https://cef-builds.spotifycdn.com/index.html\n\
             =============================================================================\n",
            cef_root
                .canonicalize()
                .unwrap_or_else(|_| cef_root.clone())
                .display()
        );
    }

    if !framework_binary.exists() {
        panic!(
            "\n=============================================================================\n\
             CEF framework binary not found at:\n\
             {}\n\
             \n\
             The CEF distribution appears incomplete.\n\
             \n\
             To download and setup CEF, run:\n\
             ./scripts/setup-cef.sh\n\
             =============================================================================\n",
            framework_binary.display()
        );
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

    eprintln!("Building CEF bridge...");
    let _dst = config.build();

    // NOTE: We do NOT link CEF into the Rust binary!
    // The Rust code only spawns the arkavo-cef-renderer subprocess,
    // which is a separate C++ binary that has CEF linked.
    // This avoids loading CEF framework into the main process.

    eprintln!("CEF bridge built successfully");
}
