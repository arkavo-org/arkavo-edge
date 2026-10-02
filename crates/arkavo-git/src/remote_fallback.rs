use crate::{GitError, Result};
use arkavo_process_env::ChildEnv;
use std::path::Path;
use std::process::Command;

/// The tokens `gh` and its git credential helper (`gh auth git-credential`)
/// authenticate a GitHub remote with when the operator exported one instead
/// of running `gh auth login`.
pub const GITHUB_CREDENTIALS: &[&str] = &[
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];

/// `git` under the tool environment. Hooks, `core.sshCommand` and
/// credential helpers all run as its children, so none of them may see the
/// agent's provider keys; the GitHub token stays so an HTTPS push through
/// `gh`'s credential helper still authenticates.
fn git_command() -> Command {
    ChildEnv::tool_from_current(GITHUB_CREDENTIALS).command("git")
}

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
    let output = git_command()
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
    let output = git_command()
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
    let output = git_command()
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
    git_command()
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::env_probe::{self, KEPT_LINE, PLANTED};
    use arkavo_test_macros::spec;

    /// The half of the regression test below that runs in the re-run
    /// process: the fallback runs a fake `git` that records its environment.
    #[test]
    fn git_env_probe() {
        let Some(dir) = env_probe::probe_dir() else {
            return;
        };
        env_probe::fake_program(&dir, "git");
        git_fallback_fetch(&dir, "origin").expect("fake git runs");
    }

    #[spec("MCP-016")]
    #[test]
    fn git_fallback_child_sees_no_provider_key_but_keeps_the_github_token() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let seen = env_probe::rerun(
            "remote_fallback::tests::git_env_probe",
            dir.path(),
            &[("GH_TOKEN", "gh-token-kept")],
            "git",
        );
        assert!(seen.lines().any(|l| l == KEPT_LINE), "{seen}");
        assert!(
            seen.lines().any(|l| l == "GH_TOKEN=gh-token-kept"),
            "{seen}"
        );
        assert!(!seen.contains(PLANTED), "{seen}");
    }
}
