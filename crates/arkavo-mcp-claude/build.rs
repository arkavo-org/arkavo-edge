use anyhow::Result;

fn main() -> Result<()> {
    // For Homebrew distribution, we don't install npm dependencies at build time
    // The capability will check for Claude Code SDK at runtime and provide instructions
    println!("cargo:rerun-if-changed=node/package.json");
    println!("cargo:rerun-if-changed=node/claude-code-bridge.js");

    // Always compile the capability, but it will be disabled at runtime if dependencies are missing
    println!(
        "cargo:warning=Claude Code capability compiled. Runtime dependencies will be checked when enabled."
    );

    Ok(())
}
