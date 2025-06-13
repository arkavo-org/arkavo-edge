use crate::{Result, TestError};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct XcodeVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl XcodeVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn detect() -> Result<Self> {
        let output = Command::new("xcodebuild")
            .arg("-version")
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run xcodebuild: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to get Xcode version: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let version_str = String::from_utf8_lossy(&output.stdout);

        // Parse "Xcode X.Y.Z" or "Xcode X.Y"
        if let Some(version_line) = version_str.lines().find(|line| line.starts_with("Xcode")) {
            let parts: Vec<&str> = version_line.split_whitespace().collect();
            if parts.len() >= 2 {
                let version_nums: Vec<&str> = parts[1].split('.').collect();
                let major = version_nums
                    .first()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let minor = version_nums
                    .get(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let patch = version_nums
                    .get(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);

                return Ok(Self::new(major, minor, patch));
            }
        }

        Err(TestError::Mcp("Failed to parse Xcode version".to_string()))
    }

    pub fn supports_bootstatus(&self) -> bool {
        // bootstatus was added in Xcode 11
        self.major >= 11
    }

    pub fn supports_privacy(&self) -> bool {
        // privacy commands were added in Xcode 11.4
        self.major > 11 || (self.major == 11 && self.minor >= 4)
    }

    pub fn supports_ui_commands(&self) -> bool {
        // UI commands were added in Xcode 15
        self.major >= 15
    }

    pub fn supports_device_appearance(&self) -> bool {
        // Device appearance commands were added in Xcode 13
        self.major >= 13
    }

    pub fn supports_push_notification(&self) -> bool {
        // Push notification support was added in Xcode 11.4
        self.major > 11 || (self.major == 11 && self.minor >= 4)
    }

    pub fn supports_clone(&self) -> bool {
        // Clone was added in Xcode 12
        self.major >= 12
    }

    pub fn supports_device_pair(&self) -> bool {
        // Device pairing was added in Xcode 14
        self.major >= 14
    }

    pub fn supports_device_focus(&self) -> bool {
        // Device focus mode was added in Xcode 16
        self.major >= 16
    }

    pub fn supports_device_streaming(&self) -> bool {
        // Device streaming was added in Xcode 25
        self.major >= 25
    }

    pub fn supports_enhanced_ui_interaction(&self) -> bool {
        // Enhanced UI interaction was added in Xcode 26
        self.major >= 26
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parsing() {
        let v = XcodeVersion::new(15, 2, 0);
        assert!(v.supports_ui_commands());
        assert!(v.supports_bootstatus());
        assert!(!v.supports_enhanced_ui_interaction());
    }

    #[test]
    fn test_version_comparison() {
        let v1 = XcodeVersion::new(15, 0, 0);
        let v2 = XcodeVersion::new(16, 0, 0);
        assert!(v1 < v2);
    }
}
