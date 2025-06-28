use crate::{GitError, Result};
use std::path::Path;
use std::process::Command;

/// Check if a URL requires HTTPS transport
pub fn is_https_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// Check if a URL is SSH
pub fn is_ssh_url(url: &str) -> bool {
    url.starts_with("ssh://") || url.starts_with("git@") || url.contains(':')
}

/// Fallback to system git for remote operations when HTTPS support is not compiled in
pub fn git_fallback_fetch(repo_path: &Path, remote: &str) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["fetch", remote])
        .output()
        .map_err(GitError::Io)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::Git(git2::Error::from_str(&format!(
            "git fetch failed: {stderr}"
        ))));
    }

    Ok(())
}

/// Fallback to system git for push operations
pub fn git_fallback_push(repo_path: &Path, remote: &str, branch: &str) -> Result<()> {
    let refspec = format!("refs/heads/{branch}");
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["push", remote, &refspec])
        .output()
        .map_err(GitError::Io)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::Git(git2::Error::from_str(&format!(
            "git push failed: {stderr}"
        ))));
    }

    Ok(())
}

/// Fallback to system git for pull operations
pub fn git_fallback_pull(repo_path: &Path, remote: &str, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .args(["pull", remote, branch])
        .output()
        .map_err(GitError::Io)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::Git(git2::Error::from_str(&format!(
            "git pull failed: {stderr}"
        ))));
    }

    Ok(())
}

/// Check if system git is available
pub fn has_system_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
