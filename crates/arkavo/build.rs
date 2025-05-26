use std::process::Command;

fn main() {
    // Get git commit hash
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .unwrap();
    
    let git_hash = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .to_string();
    
    // Set as environment variable for compile time
    println!("cargo:rustc-env=GIT_COMMIT_HASH={}", git_hash);
    
    // Rerun if .git/HEAD changes
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}