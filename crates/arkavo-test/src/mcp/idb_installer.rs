use crate::{Result, TestError};
use std::path::PathBuf;
use std::process::Command;

/// Handles installation of IDB from official sources
pub struct IdbInstaller;

impl IdbInstaller {
    /// Check if IDB is properly installed via brew
    pub fn is_idb_installed() -> bool {
        // Check if idb_companion exists in standard locations
        let standard_paths = [
            "/opt/homebrew/bin/idb_companion", // Apple Silicon homebrew
        ];

        for path in &standard_paths {
            if std::path::Path::new(path).exists() {
                // Verify it can run
                if let Ok(output) = Command::new(path).arg("--help").output() {
                    if output.status.success() {
                        return true;
                    }
                }
            }
        }

        // Check if available via PATH
        if let Ok(output) = Command::new("which").arg("idb_companion").output() {
            if output.status.success() {
                return true;
            }
        }

        false
    }

    /// Get the path to a working idb_companion
    pub fn get_idb_path() -> Option<PathBuf> {
        // First check brew installations for idb_companion
        let standard_paths = [
            "/opt/homebrew/bin/idb_companion", // Apple Silicon homebrew
        ];

        for path_str in &standard_paths {
            let path = PathBuf::from(path_str);
            if path.exists() {
                // Verify it can run
                if let Ok(output) = Command::new(&path).arg("--help").output() {
                    if output.status.success() {
                        eprintln!(
                            "[IdbInstaller] Found working idb_companion at: {}",
                            path.display()
                        );
                        return Some(path);
                    }
                }
            }
        }

        // Check PATH for idb_companion
        if let Ok(output) = Command::new("which").arg("idb_companion").output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    let path = PathBuf::from(path_str);
                    eprintln!(
                        "[IdbInstaller] Found idb_companion in PATH at: {}",
                        path.display()
                    );
                    return Some(path);
                }
            }
        }

        // Fall back to Python idb CLI if available
        let idb_paths = ["/usr/local/bin/idb", "/opt/homebrew/bin/idb"];

        for path_str in &idb_paths {
            let path = PathBuf::from(path_str);
            if path.exists() {
                // Verify it's the Facebook IDB
                if let Ok(output) = Command::new(&path).arg("--help").output() {
                    if output.status.success() {
                        let help_text = String::from_utf8_lossy(&output.stdout);
                        if help_text.contains("iOS Simulators and Devices") {
                            eprintln!("[IdbInstaller] Found Python idb CLI at: {}", path.display());
                            return Some(path);
                        }
                    }
                }
            }
        }

        // Check PATH for Python idb
        if let Ok(output) = Command::new("which").arg("idb").output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path_str.is_empty() {
                    let path = PathBuf::from(path_str);
                    // Verify it's the Facebook IDB
                    if let Ok(help_output) = Command::new(&path).arg("--help").output() {
                        let help_text = String::from_utf8_lossy(&help_output.stdout);
                        if help_text.contains("iOS Simulators and Devices") {
                            eprintln!(
                                "[IdbInstaller] Found Python idb CLI in PATH at: {}",
                                path.display()
                            );
                            return Some(path);
                        }
                    }
                }
            }
        }

        None
    }

    /// Install IDB using brew (for user guidance only)
    pub fn get_install_instructions() -> String {
        "IDB (iOS Development Bridge) is required but not installed properly.\n\n\
         To install IDB:\n\
         1. Install Homebrew if not already installed: https://brew.sh\n\
         2. Run: brew tap facebook/fb\n\
         3. Run: brew install facebook/fb/idb-companion\n\n\
         This installs the official IDB with all required frameworks.\n\
         After installation, restart the calibration process."
            .to_string()
    }

    /// Check if brew is available
    pub fn is_brew_available() -> bool {
        Command::new("brew")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Attempt to auto-install IDB (returns instructions if can't auto-install)
    pub async fn attempt_auto_install() -> Result<String> {
        if !Self::is_brew_available() {
            return Ok(Self::get_install_instructions());
        }

        eprintln!("[IdbInstaller] Attempting to install IDB via brew...");

        // First, tap the Facebook repository
        let tap_output = Command::new("brew")
            .args(&["tap", "facebook/fb"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run brew tap: {}", e)))?;

        if !tap_output.status.success() {
            return Ok(format!(
                "Failed to tap Facebook brew repository:\n{}\n\n{}",
                String::from_utf8_lossy(&tap_output.stderr),
                Self::get_install_instructions()
            ));
        }

        // Install idb-companion
        eprintln!("[IdbInstaller] Installing idb-companion via brew...");
        let install_output = Command::new("brew")
            .args(&["install", "facebook/fb/idb-companion"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run brew install: {}", e)))?;

        if !install_output.status.success() {
            return Ok(format!(
                "Failed to install idb-companion:\n{}\n\n{}",
                String::from_utf8_lossy(&install_output.stderr),
                Self::get_install_instructions()
            ));
        }

        eprintln!("[IdbInstaller] IDB installation completed successfully");

        // Verify installation
        if Self::is_idb_installed() {
            Ok("IDB has been successfully installed via brew.".to_string())
        } else {
            Ok(
                "IDB installation completed but verification failed. Please check manually."
                    .to_string(),
            )
        }
    }
}
