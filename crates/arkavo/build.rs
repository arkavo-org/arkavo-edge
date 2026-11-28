use std::process::Command;

fn main() {
    // Get git commit hash if git is available
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Set as environment variable for compile time
    println!("cargo:rustc-env=GIT_COMMIT_HASH={git_hash}");

    // Rerun if .git/HEAD changes (if it exists)
    println!("cargo:rerun-if-changed=../../.git/HEAD");

    // On macOS, aws-lc-fips-sys links dynamically against
    // `@rpath/libaws_lc_fips_0_14_4_crypto.dylib`. Embed executable-relative
    // rpaths so packaged binaries resolve the bundled FIPS dylib without
    // requiring DYLD_LIBRARY_PATH.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" && std::env::var_os("CARGO_FEATURE_CRYPTO_PROFILE").is_some() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/../lib");
    }
}
