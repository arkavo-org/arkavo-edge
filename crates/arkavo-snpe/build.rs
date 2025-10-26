fn main() {
    if cfg!(not(all(target_os = "linux", target_arch = "aarch64"))) {
        println!("cargo:warning=arkavo-snpe is only supported on aarch64-linux (UNO Q platform)");
        return;
    }

    println!("cargo:warning=SNPE SDK will be loaded dynamically at runtime via dlopen");
    println!("cargo:warning=No build-time linking required - binary is portable");
    println!("cargo:warning=Runtime will search: /opt/snpe/lib, $SNPE_ROOT/lib, $LD_LIBRARY_PATH");
}
