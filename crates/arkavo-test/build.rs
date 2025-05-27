use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();

    // Compile appropriate implementation based on platform
    match target_os.as_str() {
        "macos" | "ios" => {
            // Use real implementation on Apple platforms
            cc::Build::new()
                .file("src/bridge/ios_impl.c")
                .flag("-framework")
                .flag("CoreFoundation")
                .warnings(true)
                .compile("ios_bridge");
        }
        _ => {
            // Use stub on other platforms
            cc::Build::new()
                .file("src/bridge/ios_stub.c")
                .warnings(false)
                .compile("ios_bridge");
        }
    }
}
