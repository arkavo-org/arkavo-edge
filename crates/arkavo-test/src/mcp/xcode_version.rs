use std::process::Command;
use std::sync::OnceLock;

static XCODE_VERSION_CACHE: OnceLock<Option<XcodeVersion>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct XcodeVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl XcodeVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn detect() -> Option<Self> {
        XCODE_VERSION_CACHE
            .get_or_init(|| Self::detect_uncached())
            .clone()
    }

    fn detect_uncached() -> Option<Self> {
        // Try to run xcodebuild without triggering system prompts
        let output = match Command::new("xcodebuild").arg("-version").output() {
            Ok(output) => output,
            Err(_) => {
                // xcodebuild not found or not executable - return None without error
                return None;
            }
        };

        if !output.status.success() {
            return None;
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

                return Some(Self::new(major, minor, patch));
            }
        }

        None
    }

    pub fn is_available() -> bool {
        Self::detect().is_some()
    }

    pub fn require_xcode_message() -> &'static str {
        "This feature requires Xcode Command Line Tools. To install:\n\
         1. Open Terminal\n\
         2. Run: xcode-select --install\n\
         3. Follow the installation prompts\n\
         \n\
         Alternatively, install the full Xcode IDE from the Mac App Store."
    }

    pub const fn supports_bootstatus(&self) -> bool {
        // bootstatus was added in Xcode 11
        self.major >= 11
    }

    pub const fn supports_privacy(&self) -> bool {
        // privacy commands were added in Xcode 11.4
        self.major > 11 || (self.major == 11 && self.minor >= 4)
    }

    pub const fn supports_ui_commands(&self) -> bool {
        // UI commands were added in Xcode 15
        self.major >= 15
    }

    pub const fn supports_device_appearance(&self) -> bool {
        // Device appearance commands were added in Xcode 13
        self.major >= 13
    }

    pub const fn supports_push_notification(&self) -> bool {
        // Push notification support was added in Xcode 11.4
        self.major > 11 || (self.major == 11 && self.minor >= 4)
    }

    pub const fn supports_clone(&self) -> bool {
        // Clone was added in Xcode 12
        self.major >= 12
    }

    pub const fn supports_device_pair(&self) -> bool {
        // Device pairing was added in Xcode 14
        self.major >= 14
    }

    pub const fn supports_device_focus(&self) -> bool {
        // Device focus mode was added in Xcode 16
        self.major >= 16
    }

    pub const fn supports_device_streaming(&self) -> bool {
        // Device streaming was added in Xcode 25
        self.major >= 25
    }

    pub const fn supports_enhanced_ui_interaction(&self) -> bool {
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
