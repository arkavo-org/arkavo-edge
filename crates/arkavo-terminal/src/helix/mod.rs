use anyhow::{Result, anyhow};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

pub struct HelixEditor {
    binary_path: Option<PathBuf>,
    scratch_dir: PathBuf,
}

impl HelixEditor {
    pub fn new() -> Result<Self> {
        let scratch_dir = std::env::temp_dir().join("arkavo-helix");
        fs::create_dir_all(&scratch_dir)?;

        Ok(Self {
            binary_path: Self::find_helix_binary(),
            scratch_dir,
        })
    }

    fn find_helix_binary() -> Option<PathBuf> {
        // First check if helix is bundled with arkavo
        if let Ok(exe_path) = std::env::current_exe()
            && let Some(exe_dir) = exe_path.parent() {
                let bundled_helix = exe_dir.join("helix").join("hx");
                if bundled_helix.exists() {
                    return Some(bundled_helix);
                }

                // Also check direct binary
                let bundled_hx = exe_dir.join("hx");
                if bundled_hx.exists() {
                    return Some(bundled_hx);
                }
            }

        // Check if helix is in PATH
        if let Ok(output) = Command::new("which").arg("hx").output()
            && output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }

        None
    }

    pub fn is_available(&self) -> bool {
        self.binary_path.is_some()
    }

    pub fn launch_with_content(&self, content: &str) -> Result<String> {
        let binary = self
            .binary_path
            .as_ref()
            .ok_or_else(|| anyhow!("Helix binary not found"))?;

        // Create a temporary file for the content
        let mut temp_file = NamedTempFile::new_in(&self.scratch_dir)?;
        temp_file.write_all(content.as_bytes())?;
        temp_file.flush()?;

        let temp_path = temp_file.path().to_path_buf();

        // Launch helix with the temporary file
        let status = Command::new(binary)
            .arg(&temp_path)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;

        if !status.success() {
            return Err(anyhow!("Helix exited with non-zero status"));
        }

        // Read back the content
        let edited_content = fs::read_to_string(&temp_path)?;

        Ok(edited_content)
    }

    pub fn launch_empty(&self) -> Result<String> {
        self.launch_with_content("")
    }

    pub fn get_binary_info(&self) -> Option<String> {
        self.binary_path.as_ref().map(|p| p.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helix_detection() {
        let helix = HelixEditor::new().unwrap();
        // This test will pass/fail based on whether helix is installed
        println!("Helix available: {}", helix.is_available());
        if let Some(path) = helix.get_binary_info() {
            println!("Helix binary at: {path}");
        }
    }
}
