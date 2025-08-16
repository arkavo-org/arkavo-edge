use std::env;
use std::path::PathBuf;

fn main() {
    // 1. Build llama.cpp using cmake
    let dst = cmake::build("/Users/paul/Projects/arkavo/arkavo-edge/vendor/llama.cpp");

    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=dylib=llama");

    // Add link libraries for different platforms
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Accelerate");
        println!("cargo:rustc-link-lib=framework=Metal");
    }

    // 2. Generate bindings
    let bindings = bindgen::Builder::default()
        .header("/Users/paul/Projects/arkavo/arkavo-edge/vendor/llama.cpp/include/llama.h")
        .clang_arg("-I/Users/paul/Projects/arkavo/arkavo-edge/vendor/llama.cpp/ggml/include")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
