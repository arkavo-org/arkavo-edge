use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=swift/secure_enclave.swift");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH");
    let arch = match target_arch.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("arkavo-secure-enclave does not build for macOS on {other}"),
    };
    // The same floor rustc links the rest of the binary against, so the Swift
    // objects never ask for a newer macOS than the program does.
    let floor = std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "11.0".to_string());
    let library = out.join("libarkavo_secure_enclave.a");

    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-O",
        ])
        .args(["-module-name", "ArkavoSecureEnclave"])
        .args(["-target", &format!("{arch}-apple-macos{floor}")])
        .arg("swift/secure_enclave.swift")
        .arg("-o")
        .arg(&library)
        .status()
        .unwrap_or_else(|e| panic!("{}: {e}", missing_swift()));
    assert!(
        status.success(),
        "swiftc failed to build the Secure Enclave shim"
    );

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=arkavo_secure_enclave");
    println!("cargo:rustc-link-lib=framework=CryptoKit");
    println!("cargo:rustc-link-lib=framework=Security");
    println!("cargo:rustc-link-lib=framework=Foundation");
    // The Swift runtime ships with macOS in /usr/lib/swift, and its dylibs
    // carry those absolute install names; the SDK carries their link stubs,
    // and the toolchain the static compatibility shims that Swift objects
    // built for an older macOS ask for.
    println!(
        "cargo:rustc-link-search=native={}",
        Path::new(&xcrun(&["--show-sdk-path"]))
            .join("usr/lib/swift")
            .display()
    );
    let swiftc = PathBuf::from(xcrun(&["--find", "swiftc"]));
    if let Some(bin) = swiftc.parent() {
        println!(
            "cargo:rustc-link-search=native={}",
            bin.join("../lib/swift/macosx").display()
        );
    }
}

fn xcrun(args: &[&str]) -> String {
    let output = Command::new("xcrun")
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{}: {e}", missing_swift()));
    assert!(output.status.success(), "{}", missing_swift());
    String::from_utf8(output.stdout)
        .expect("xcrun printed a path that is not UTF-8")
        .trim()
        .to_string()
}

fn missing_swift() -> &'static str {
    "building for macOS needs the Swift compiler from Xcode or the Command Line Tools \
     (xcode-select --install)"
}
