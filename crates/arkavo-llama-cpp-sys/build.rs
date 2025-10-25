use std::env;
use std::path::PathBuf;

fn main() {
    // Skip building for musl targets - llama.cpp doesn't work well with musl
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("musl") {
        println!("cargo:warning=Skipping llama.cpp build for musl target");
        // Create dummy bindings for musl
        let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
        std::fs::write(out_path.join("bindings.rs"), "// Dummy bindings for musl\n").unwrap();
        return;
    }
    // Track the actual header file location (relative to crate root)
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/include/llama.h");

    // Track key source directories
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/src");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/ggml");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/CMakeLists.txt");

    // Track this build script itself
    println!("cargo:rerun-if-changed=build.rs");

    let mut config = cmake::Config::new("../../vendor/llama.cpp");

    // Determine build type based on profile
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let build_type = if profile == "release" {
        "Release" // Full optimizations, no debug info
    } else {
        "MinSizeRel" // Optimize for size in debug builds, much faster to compile
    };

    config
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CMAKE_BUILD_TYPE", build_type);

    // Enable parallel compilation (skip -j for Windows/MSBuild)
    if !target.contains("windows") {
        if let Ok(num_jobs) = env::var("NUM_JOBS") {
            config.build_arg(format!("-j{}", num_jobs));
        } else {
            // Use number of CPUs
            let num_cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            config.build_arg(format!("-j{}", num_cpus));
        }
    }
    // MSBuild handles parallelism automatically with /m flag (set by cmake internally)

    // Platform-specific GPU acceleration
    if cfg!(target_os = "macos") {
        config
            .define("GGML_METAL", "ON") // Enable Metal for GPU on macOS
            .define("GGML_ACCELERATE", "ON") // use Apple Accelerate
            .define("GGML_NATIVE", "OFF") // Disable native CPU feature detection to ensure compatibility
            .define("GGML_CPU_ARM_ARCH", "armv8.2-a+fp16"); // Use baseline ARM arch without i8mm

        // Disable Metal debug overhead in release builds
        let is_release = env::var("PROFILE").unwrap_or_default() == "release";
        if is_release {
            config.define("GGML_METAL_NDEBUG", "ON");
        }
    } else if cfg!(target_os = "windows") {
        // Windows can use Vulkan or CUDA if available
        config
            .define("GGML_VULKAN", "OFF") // Could be enabled if Vulkan SDK present
            .define("GGML_CUDA", "OFF"); // Could be enabled if CUDA toolkit present
    } else {
        // Linux - use CPU optimizations by default with static linking
        config
            .define("GGML_BLAS", "OFF") // Could be enabled if OpenBLAS present
            .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON") // Required for static linking
            .define("GGML_STATIC", "ON") // Build static libraries
            .define("GGML_OPENMP", "ON"); // Enable OpenMP but link statically

        // ARM64-specific optimizations for Raspberry Pi and similar systems
        if target.contains("aarch64") || target.contains("arm64") {
            config
                .define("GGML_NATIVE", "OFF") // Disable native CPU feature detection for portability
                .define("GGML_CPU_ARM_ARCH", "armv8.2-a+fp16"); // Baseline ARMv8.2 with FP16 support

            // Enable both OpenCL and Vulkan for Qualcomm Adreno GPU (UNO Q, Android boards)
            // OpenCL: Actively maintained by Qualcomm, with Adreno-specific optimizations
            // Vulkan: Mesa Turnip driver for Adreno 702 (experimental)
            eprintln!("Enabling OpenCL + Vulkan GPU acceleration for aarch64-linux (Adreno support)");
            config.define("GGML_OPENCL", "ON");
            config.define("GGML_OPENCL_USE_ADRENO_KERNELS", "ON"); // Use optimized Adreno kernels
            config.define("GGML_VULKAN", "ON");
        } else {
            // Disable OpenCL on non-ARM64 platforms
            config.define("GGML_OPENCL", "OFF");
        }
    }

    // Common settings for all platforms
    config
        .define("GGML_ASSERTS", "OFF") // Disable asserts for performance
        .define("LLAMA_CURL", "OFF") // Disable CURL requirement (not needed for local inference)
        .define("LLAMA_BUILD_TESTS", "OFF") // Don't build tests
        .define("LLAMA_BUILD_EXAMPLES", "OFF") // Don't build examples
        .define("LLAMA_BUILD_SERVER", "OFF"); // Don't build server

    // Use ccache if available for faster rebuilds
    if let Ok(ccache) = which::which("ccache") {
        config
            .define("CMAKE_C_COMPILER_LAUNCHER", ccache.to_str().unwrap())
            .define("CMAKE_CXX_COMPILER_LAUNCHER", ccache.to_str().unwrap());
        eprintln!("Using ccache for faster C++ compilation");
    }

    let dst = config.build();

    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // Dynamically discover and link all static libraries built by CMake
    // This handles both Unix (.a) and Windows (.lib) naming conventions
    if cfg!(target_os = "windows") {
        // Windows: look for *.lib files
        for entry in std::fs::read_dir(&lib_dir)
            .unwrap_or_else(|_| panic!("Failed to read lib directory: {}", lib_dir.display()))
        {
            let entry = entry.unwrap();
            let path = entry.path();
            if let Some(fname) = path.file_name().and_then(|f| f.to_str()) {
                if fname.ends_with(".lib") && !fname.ends_with(".dll.lib") {
                    let libname = fname.trim_end_matches(".lib");
                    println!("cargo:rustc-link-lib=static={}", libname);
                }
            }
        }
    } else {
        // Unix: look for lib*.a files
        for entry in std::fs::read_dir(&lib_dir)
            .unwrap_or_else(|_| panic!("Failed to read lib directory: {}", lib_dir.display()))
        {
            let entry = entry.unwrap();
            let path = entry.path();
            if let Some(fname) = path.file_name().and_then(|f| f.to_str()) {
                if fname.starts_with("lib") && fname.ends_with(".a") {
                    let libname = fname.trim_start_matches("lib").trim_end_matches(".a");
                    println!("cargo:rustc-link-lib=static={}", libname);
                }
            }
        }
    }

    // Platform-specific linking
    if cfg!(target_os = "macos") {
        // Metal backend for GPU acceleration on macOS
        println!("cargo:rustc-link-lib=static=ggml-metal");
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=MetalKit");
        println!("cargo:rustc-link-lib=framework=MetalPerformanceShaders");
        println!("cargo:rustc-link-lib=framework=Accelerate");
        println!("cargo:rustc-link-lib=framework=Foundation");
    } else if cfg!(target_os = "windows") {
        // Windows MSVC uses automatic C++ runtime linking via /MD or /MT flag
        // CMake handles this automatically based on CMAKE_MSVC_RUNTIME_LIBRARY
        // No explicit cargo:rustc-link-lib needed for C++ runtime
    } else {
        // Linux specific libraries
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m"); // math library

        // Static linking of OpenMP for zero-config deployment
        let target = env::var("TARGET").unwrap_or_default();
        if target.contains("aarch64") || target.contains("arm64") {
            // ARM64 library paths for cross-compilation and native builds
            // Try multiple GCC versions (Ubuntu 22.04 typically has GCC 11-12)
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/13");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/12");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/11");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/10");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/aarch64-linux-gnu/9");
            println!("cargo:rustc-link-search=native=/usr/lib/aarch64-linux-gnu");
            println!("cargo:rustc-link-search=native=/usr/aarch64-linux-gnu/lib");
            // Also check cross-compilation sysroot
            println!("cargo:rustc-link-search=native=/usr/lib/gcc-cross/aarch64-linux-gnu/13");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc-cross/aarch64-linux-gnu/12");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc-cross/aarch64-linux-gnu/11");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc-cross/aarch64-linux-gnu/10");

            // Vulkan support for Adreno GPU (UNO Q with Mesa Turnip driver)
            println!("cargo:rustc-link-lib=vulkan");
        } else {
            // x86_64 library paths
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/13");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/12");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/11");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/10");
            println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/9");
            println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");
        }
        println!("cargo:rustc-link-lib=static=gomp"); // Static OpenMP
    }

    // C++ standard library (handling varies by platform)
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    }

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let bindings_path = out_path.join("bindings.rs");
    let header = out_path.join("include").join("llama.h");

    // Check if header exists
    if !header.exists() {
        panic!(
            "llama.h not found at {:?}. CMake build may have failed.",
            header
        );
    }

    // Only regenerate bindings if they don't exist or header changed
    let should_regenerate = !bindings_path.exists() || {
        let header_mtime = std::fs::metadata(&header).and_then(|m| m.modified()).ok();
        let bindings_mtime = std::fs::metadata(&bindings_path)
            .and_then(|m| m.modified())
            .ok();

        match (header_mtime, bindings_mtime) {
            (Some(h), Some(b)) => h > b,
            _ => true,
        }
    };

    if should_regenerate {
        eprintln!("Generating Rust bindings for llama.cpp");
        let bindings = bindgen::Builder::default()
            .header(header.to_str().unwrap())
            .clang_arg(format!("-I{}", out_path.join("include").display()))
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
            .expect("Unable to generate bindings");

        bindings
            .write_to_file(&bindings_path)
            .expect("Couldn't write bindings!");
    }
}
