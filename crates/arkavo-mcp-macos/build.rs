use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();

    match target_os.as_str() {
        "macos" | "ios" => {
            cc::Build::new()
                .file("src/bridge/ios_impl.c")
                .warnings(true)
                .compile("ios_bridge");

            println!("cargo:rustc-link-lib=framework=CoreFoundation");
        }
        "windows" => {
            cc::Build::new()
                .file("src/bridge/windows_stub.c")
                .warnings(false)
                .compile("ios_bridge");

            println!("cargo:warning=iOS testing capabilities are not available on Windows");
        }
        _ => {
            cc::Build::new()
                .file("src/bridge/ios_stub.c")
                .warnings(false)
                .compile("ios_bridge");
        }
    }
}
