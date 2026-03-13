use crate::error::Result;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tracing::debug;

/// Cross-platform symlink: uses symlinks on Unix, copies on Windows.
fn portable_link(src: &Path, dest: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dest)
    }
    #[cfg(not(unix))]
    {
        if src.is_dir() {
            copy_dir_fallback(src, dest)
        } else {
            std::fs::copy(src, dest).map(|_| ())
        }
    }
}

#[cfg(not(unix))]
fn copy_dir_fallback(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            if entry.file_name() == "target" {
                continue;
            }
            copy_dir_fallback(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

/// An isolated temp workspace for compiling and testing modified source.
///
/// Symlinks workspace-level files and sibling crate directories into a temp
/// directory. Does NOT symlink `target/` — the temp workspace gets its own
/// build cache to avoid mutation risk to the real build.
pub struct TempWorkspace {
    dir: TempDir,
    project_root: PathBuf,
}

/// Result of running compile/test checks in the temp workspace.
#[derive(Debug)]
pub struct VerificationResult {
    pub compile_ok: bool,
    pub test_ok: bool,
    pub compile_stderr: String,
    pub test_stderr: String,
    pub duration: Duration,
}

impl TempWorkspace {
    /// Create a temp workspace by symlinking from the project root.
    ///
    /// Symlinks: Cargo.toml, Cargo.lock, .cargo/, .clippy.toml, and all
    /// directories in crates/. The target crate is copied (not symlinked)
    /// so its source can be modified.
    pub fn from_project(project_root: &Path, target_crate: &str) -> Result<Self> {
        let dir = TempDir::new()?;
        let ws = dir.path();

        // Symlink workspace-level files
        for name in &[
            "Cargo.toml",
            "Cargo.lock",
            ".cargo",
            ".clippy.toml",
            "rust-toolchain.toml",
        ] {
            let src = project_root.join(name);
            if src.exists() {
                portable_link(&src, &ws.join(name))?;
            }
        }

        // Create crates/ dir
        let crates_dir = ws.join("crates");
        std::fs::create_dir_all(&crates_dir)?;

        // Symlink all sibling crates, copy the target crate
        let src_crates = project_root.join("crates");
        if src_crates.is_dir() {
            for entry in std::fs::read_dir(&src_crates)? {
                let entry = entry?;
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let dest = crates_dir.join(&name);

                if name_str == target_crate {
                    copy_dir_recursive(&entry.path(), &dest)?;
                } else {
                    portable_link(&entry.path(), &dest)?;
                }
            }
        }

        // Symlink tests/ and examples/ if they exist (workspace may reference them)
        for name in &["tests", "examples"] {
            let src = project_root.join(name);
            if src.exists() {
                portable_link(&src, &ws.join(name))?;
            }
        }

        debug!(
            workspace = %ws.display(),
            target_crate,
            "created temp workspace"
        );

        Ok(Self {
            dir,
            project_root: project_root.to_path_buf(),
        })
    }

    /// Write modified source into the temp workspace.
    pub fn write_modified(&self, relative_path: &str, content: &str) -> Result<()> {
        let path = self.dir.path().join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// Run `cargo check -p <crate>` in the temp workspace.
    pub async fn check(&self, crate_name: &str) -> Result<VerificationResult> {
        let start = Instant::now();
        let output = tokio::process::Command::new("cargo")
            .args(["check", "-p", crate_name, "-q"])
            .current_dir(self.dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("CARGO_TARGET_DIR", self.dir.path().join("target"))
            .output()
            .await?;

        let compile_ok = output.status.success();
        let compile_stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        Ok(VerificationResult {
            compile_ok,
            test_ok: false,
            compile_stderr,
            test_stderr: String::new(),
            duration: start.elapsed(),
        })
    }

    /// Run `cargo test -p <crate>` in the temp workspace.
    pub async fn test(&self, crate_name: &str) -> Result<VerificationResult> {
        let start = Instant::now();

        // First check compilation
        let check = self.check(crate_name).await?;
        if !check.compile_ok {
            return Ok(VerificationResult {
                compile_ok: false,
                test_ok: false,
                compile_stderr: check.compile_stderr,
                test_stderr: String::new(),
                duration: start.elapsed(),
            });
        }

        // Then run tests
        let output = tokio::process::Command::new("cargo")
            .args(["test", "-p", crate_name, "-q"])
            .current_dir(self.dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("CARGO_TARGET_DIR", self.dir.path().join("target"))
            .output()
            .await?;

        Ok(VerificationResult {
            compile_ok: true,
            test_ok: output.status.success(),
            compile_stderr: check.compile_stderr,
            test_stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            duration: start.elapsed(),
        })
    }

    /// Path to the temp workspace directory.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Path to the original project root.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}

/// Extract the crate name from a relative file path.
///
/// e.g. `"crates/arkavo-llm/src/lib.rs"` → `"arkavo-llm"`
pub fn crate_name_from_path(relative_path: &str) -> Option<&str> {
    let path = Path::new(relative_path);
    let mut components = path.components();
    // Skip "crates/"
    let first = components.next()?;
    if first.as_os_str() != "crates" {
        return None;
    }
    components.next()?.as_os_str().to_str()
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dest.join(entry.file_name());

        if file_type.is_dir() {
            // Skip target directories inside crates
            if entry.file_name() == "target" {
                continue;
            }
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), dest_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_extraction() {
        assert_eq!(
            crate_name_from_path("crates/arkavo-llm/src/lib.rs"),
            Some("arkavo-llm")
        );
        assert_eq!(
            crate_name_from_path("crates/arkavo-evofabric/src/rust_op.rs"),
            Some("arkavo-evofabric")
        );
        assert_eq!(crate_name_from_path("src/main.rs"), None);
        assert_eq!(crate_name_from_path(""), None);
    }

    #[test]
    fn temp_workspace_creation() {
        // Use the actual project root for this test
        let project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        if !project_root.join("Cargo.toml").exists() {
            return; // Skip if not running from workspace
        }

        let ws = TempWorkspace::from_project(project_root, "arkavo-evofabric").unwrap();
        assert!(ws.path().join("Cargo.toml").exists());
        assert!(ws.path().join("crates").exists());
        // Target crate should be a real directory (copied), not a symlink
        let target = ws.path().join("crates/arkavo-evofabric");
        assert!(target.exists());
        assert!(!target.is_symlink());
    }

    #[test]
    fn write_modified_creates_parents() {
        let dir = TempDir::new().unwrap();
        let ws = TempWorkspace {
            dir,
            project_root: PathBuf::from("/tmp"),
        };
        ws.write_modified("deep/nested/file.rs", "fn test() {}")
            .unwrap();
        assert!(ws.path().join("deep/nested/file.rs").exists());
    }
}
