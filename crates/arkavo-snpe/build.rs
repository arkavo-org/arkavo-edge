use std::env;
use std::path::PathBuf;

fn main() {
    if cfg!(not(all(target_os = "linux", target_arch = "aarch64"))) {
        println!("cargo:warning=arkavo-snpe is only supported on aarch64-linux (UNO Q platform)");
        return;
    }

    if let Ok(snpe_root) = env::var("SNPE_ROOT") {
        let snpe_path = PathBuf::from(&snpe_root);
        let lib_path = snpe_path.join("lib").join("aarch64-linux");

        if lib_path.exists() {
            println!("cargo:rustc-link-search=native={}", lib_path.display());
            println!("cargo:rustc-link-lib=dylib=SNPE");

            println!("cargo:rerun-if-env-changed=SNPE_ROOT");
            println!("cargo:warning=SNPE SDK found at: {snpe_root} (dynamic linking)");
        } else {
            println!(
                "cargo:warning=SNPE_ROOT is set but lib directory not found at: {}",
                lib_path.display()
            );
            println!("cargo:warning=Expected: $SNPE_ROOT/lib/aarch64-linux");
        }
    } else {
        println!(
            "cargo:warning=SNPE_ROOT not set - build will succeed but runtime requires SNPE SDK"
        );
        println!("cargo:warning=Set SNPE_ROOT to your SNPE SDK installation directory");
        println!("cargo:warning=Example: export SNPE_ROOT=/opt/snpe");
    }
}
