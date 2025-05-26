use std::env;

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    
    // Only compile the stub on non-iOS platforms
    if target_os != "ios" {
        cc::Build::new()
            .file("src/bridge/ios_stub.c")
            .warnings(false)  // Disable warnings for stub implementation
            .compile("ios_bridge_stub");
    }
}