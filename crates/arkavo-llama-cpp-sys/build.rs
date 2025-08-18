use std::env;
use std::path::PathBuf;

fn main() {
    // Track the actual header file location
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/include/llama.h");
    
    // Track key source directories
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/src");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/ggml");
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/CMakeLists.txt");
    
    // Track this build script itself
    println!("cargo:rerun-if-changed=build.rs");

    let dst = cmake::Config::new("../../vendor/llama.cpp")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CMAKE_BUILD_TYPE", "RelWithDebInfo")  // Optimized with debug info
        .define("GGML_METAL", "ON")                    // Enable Metal for GPU
        .define("GGML_CUDA", "OFF")
        .define("GGML_VULKAN", "OFF")
        .define("GGML_OPENCL", "OFF")
        .define("GGML_BLAS", "OFF")                    // avoid dual-BLAS confusion
        .define("GGML_ACCELERATE", "ON")               // use Apple Accelerate
        .define("GGML_ASSERTS", "OFF")                 // Disable asserts for performance
        .build();

    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=llama");
    println!("cargo:rustc-link-lib=static=ggml");
    println!("cargo:rustc-link-lib=static=ggml-base");
    println!("cargo:rustc-link-lib=static=ggml-cpu");
    
    // Enable Metal backend for GPU acceleration on macOS
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=static=ggml-metal");
        println!("cargo:rustc-link-lib=framework=Metal");
        println!("cargo:rustc-link-lib=framework=MetalKit");
        println!("cargo:rustc-link-lib=framework=MetalPerformanceShaders");
    }
    
    println!("cargo:rustc-link-lib=c++");
    println!("cargo:rustc-link-lib=framework=Accelerate");
    println!("cargo:rustc-link-lib=framework=Foundation");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    let header = out_path.join("include").join("llama.h");
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