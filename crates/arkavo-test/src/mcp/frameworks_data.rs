// Framework handling for IDB companion
// Since the official IDB release doesn't include frameworks, we need to handle this differently

use crate::{Result, TestError};
use std::fs;
use std::path::PathBuf;

// Check if system has IDB frameworks installed
pub fn check_system_frameworks() -> bool {
    // Check common locations where brew installs IDB frameworks
    let framework_paths = [
        "/opt/homebrew/Frameworks",
        "/usr/local/Frameworks",
        "/Library/Frameworks",
    ];

    for path in &framework_paths {
        let fbcontrol = PathBuf::from(path).join("FBControlCore.framework");
        if fbcontrol.exists() {
            return true;
        }
    }

    false
}

// Set up framework symlinks in our temp directory
pub fn setup_framework_links(target_dir: &PathBuf) -> Result<()> {
    let frameworks_dir = target_dir.join("Frameworks");
    fs::create_dir_all(&frameworks_dir)
        .map_err(|e| TestError::Mcp(format!("Failed to create Frameworks directory: {}", e)))?;

    // Find system frameworks
    let system_frameworks = [
        "/opt/homebrew/Frameworks",
        "/usr/local/Frameworks",
        "/Library/Frameworks",
    ];

    let mut found_path = None;
    for path in &system_frameworks {
        let fbcontrol = PathBuf::from(path).join("FBControlCore.framework");
        if fbcontrol.exists() {
            found_path = Some(PathBuf::from(path));
            break;
        }
    }

    if let Some(system_path) = found_path {
        // Create symlinks to system frameworks
        let frameworks = [
            "FBControlCore.framework",
            "FBDeviceControl.framework",
            "FBSimulatorControl.framework",
        ];

        for framework in &frameworks {
            let source = system_path.join(framework);
            let target = frameworks_dir.join(framework);

            if source.exists() && !target.exists() {
                eprintln!(
                    "[FrameworksData] Linking {} from {}",
                    framework,
                    source.display()
                );
                std::os::unix::fs::symlink(&source, &target).map_err(|e| {
                    TestError::Mcp(format!("Failed to create symlink for {}: {}", framework, e))
                })?;
            }
        }

        Ok(())
    } else {
        Err(TestError::Mcp(
            "IDB frameworks not found in expected system locations".to_string(),
        ))
    }
}

// For backwards compatibility
pub fn extract_frameworks(target_dir: &PathBuf) -> Result<()> {
    setup_framework_links(target_dir)
}
