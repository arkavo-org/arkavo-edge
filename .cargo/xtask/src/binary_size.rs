use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::process::Command;

const DEFAULT_BINARY: &str = "arkavo";

pub fn check(limit_mb: u64, package: Option<String>) -> Result<()> {
    let package_name = package.unwrap_or_else(|| DEFAULT_BINARY.to_string());

    let status = Command::new("cargo")
        .args(["build", "--release", "-p", &package_name])
        .status()
        .with_context(|| format!("failed to build release binary for '{package_name}'"))?;

    if !status.success() {
        let code = status.code().map_or_else(
            || "terminated by signal".to_string(),
            |c| format!("exit code {c}"),
        );
        return Err(anyhow!(
            "release build failed for package '{package_name}' ({code})"
        ));
    }

    let binary_path = release_binary_path(&package_name)?;
    let metadata = std::fs::metadata(&binary_path)
        .with_context(|| format!("missing binary: {binary_path:?}"))?;

    let size_bytes = metadata.len();
    let size_mb = size_bytes as f64 / (1024.0 * 1024.0);

    println!(
        "Binary {} size: {:.2} MiB (limit {} MiB)",
        binary_path.display(),
        size_mb,
        limit_mb
    );

    if size_mb > limit_mb as f64 {
        Err(anyhow!(
            "binary {} exceeds limit: {:.2} MiB > {} MiB",
            binary_path.display(),
            size_mb,
            limit_mb
        ))
    } else {
        Ok(())
    }
}

fn release_binary_path(package_name: &str) -> Result<PathBuf> {
    let mut path = PathBuf::from("target");
    path.push("release");

    let binary_name = if cfg!(target_os = "windows") {
        format!("{package_name}.exe")
    } else {
        package_name.to_string()
    };

    path.push(binary_name);
    Ok(path)
}
