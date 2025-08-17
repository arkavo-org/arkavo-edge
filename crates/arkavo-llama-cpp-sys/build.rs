use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=../../vendor/llama.cpp/llama.h");

    let dst = cmake::Config::new("../../vendor/llama.cpp")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("CMAKE_BUILD_TYPE", "Debug")        // or RelWithDebInfo
        .define("GGML_METAL", "OFF")                // keep off till stable
        .define("GGML_CUDA", "OFF")
        .define("GGML_VULKAN", "OFF")
        .define("GGML_OPENCL", "OFF")
        .define("GGML_BLAS", "OFF")                 // avoid dual-BLAS confusion
        .define("GGML_ACCELERATE", "ON")            // use Apple Accelerate
        .define("GGML_ASSERTS", "ON")
        .build();

    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=llama");
    println!("cargo:rustc-link-lib=static=ggml");
    println!("cargo:rustc-link-lib=static=ggml-base");
    println!("cargo:rustc-link-lib=static=ggml-cpu");
    
    // Metal backend disabled due to crash issues
    // if cfg!(target_os = "macos") {
    //     println!("cargo:rustc-link-lib=static=ggml-metal");
    // }
    
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