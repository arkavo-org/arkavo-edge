use std::env;
use std::path::PathBuf;

fn main() {
    // Track the actual header file location (relative to crate root)
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/include/llama.h");

    // Track key source directories
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/src");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/ggml");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/CMakeLists.txt");

    // Track this build script itself
    println!("cargo:rerun-if-changed=build.rs");

    let mut config = cmake::Config::new("../../vendor/llama.cpp");
    config
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CMAKE_BUILD_TYPE", "RelWithDebInfo"); // Optimized with debug info

    // Platform-specific GPU acceleration
    if cfg!(target_os = "macos") {
        config
            .define("GGML_METAL", "ON") // Enable Metal for GPU on macOS
            .define("GGML_ACCELERATE", "ON"); // use Apple Accelerate
    } else if cfg!(target_os = "windows") {
        // Windows can use Vulkan or CUDA if available
        config
            .define("GGML_VULKAN", "OFF") // Could be enabled if Vulkan SDK present
            .define("GGML_CUDA", "OFF"); // Could be enabled if CUDA toolkit present
    } else {
        // Linux - use CPU optimizations by default
        config
            .define("GGML_BLAS", "OFF") // Could be enabled if OpenBLAS present
            .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON"); // Required for static linking
    }

    // Common settings for all platforms
    config
        .define("GGML_OPENCL", "OFF")
        .define("GGML_ASSERTS", "OFF") // Disable asserts for performance
        .define("LLAMA_CURL", "OFF"); // Disable CURL requirement (not needed for local inference)

    let dst = config.build();

    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=llama");
    println!("cargo:rustc-link-lib=static=ggml");
    
    // Only link libraries that actually exist
    if lib_dir.join("libggml-base.a").exists() {
        println!("cargo:rustc-link-lib=static=ggml-base");
    }
    if lib_dir.join("libggml-cpu.a").exists() {
        println!("cargo:rustc-link-lib=static=ggml-cpu");
    }
    if lib_dir.join("libggml-blas.a").exists() {
        println!("cargo:rustc-link-lib=static=ggml-blas");
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
        // Windows specific libraries
        println!("cargo:rustc-link-lib=dylib=c++");
    } else {
        // Linux specific libraries
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m"); // math library
        println!("cargo:rustc-link-lib=gomp"); // OpenMP support
    }

    // C++ standard library (handling varies by platform)
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    }

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let header = out_path.join("include").join("llama.h");

    // Check if header exists
    if !header.exists() {
        panic!(
            "llama.h not found at {:?}. CMake build may have failed.",
            header
        );
    }

    let bindings = bindgen::Builder::default()
        .header(header.to_str().unwrap())
        .clang_arg(format!("-I{}", out_path.join("include").display()))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
