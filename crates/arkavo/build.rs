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
}
