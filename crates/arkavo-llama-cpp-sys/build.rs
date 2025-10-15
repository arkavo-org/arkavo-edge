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
    // Track the actual header file locations (relative to crate root)
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/include/llama.h");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/tools/mtmd/mtmd.h");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/tools/mtmd/clip.h");

    // Track key source directories
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/src");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/ggml");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/tools/mtmd");
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
    }

    // Common settings for all platforms
    config
        .define("GGML_OPENCL", "OFF")
        .define("GGML_ASSERTS", "OFF") // Disable asserts for performance
        .define("LLAMA_CURL", "OFF") // Disable CURL requirement (not needed for local inference)
        .define("LLAMA_BUILD_MTMD", "ON"); // Enable multimodal support

    let dst = config.build();

    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=llama");
    println!("cargo:rustc-link-lib=static=ggml");

    // Link multimodal library if it exists
    if lib_dir.join("libmtmd.a").exists() || lib_dir.join("mtmd.lib").exists() {
        println!("cargo:rustc-link-lib=static=mtmd");
    }

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

        // Static linking of OpenMP for zero-config deployment
        // Try multiple possible GCC library paths
        println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/11");
        println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/10");
        println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/9");
        println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");
        println!("cargo:rustc-link-lib=static=gomp"); // Static OpenMP
    }

    // C++ standard library (handling varies by platform)
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    }

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let header = out_path.join("include").join("llama.h");
    let mtmd_header = out_path.join("include").join("mtmd.h");

    // Check if main header exists
    if !header.exists() {
        panic!(
            "llama.h not found at {:?}. CMake build may have failed.",
            header
        );
    }

    // Create a wrapper header that includes both llama.h and mtmd.h
    let wrapper_header = out_path.join("wrapper.h");
    let mut wrapper_content = format!("#include \"{}\"\n", header.display());
    if mtmd_header.exists() {
        wrapper_content.push_str(&format!("#include \"{}\"\n", mtmd_header.display()));
    }
    std::fs::write(&wrapper_header, wrapper_content).expect("Failed to write wrapper header");

    let bindings = bindgen::Builder::default()
        .header(wrapper_header.to_str().unwrap())
        .clang_arg(format!("-I{}", out_path.join("include").display()))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_function("llama_.*")
        .allowlist_function("ggml_.*")
        .allowlist_function("mtmd_.*")
        .allowlist_function("clip_.*")
        .allowlist_type("llama_.*")
        .allowlist_type("ggml_.*")
        .allowlist_type("mtmd_.*")
        .allowlist_type("clip_.*")
        .allowlist_var("LLAMA_.*")
        .allowlist_var("GGML_.*")
        .allowlist_var("MTMD_.*")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
