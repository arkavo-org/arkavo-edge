/// iOS version parsing and detection utilities
#[derive(Debug, Clone, PartialEq)]
pub struct IosVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: Option<u32>,
    pub is_beta: bool,
}

impl IosVersion {
    /// Parse iOS version from runtime string
    /// Examples: "iOS-17-4", "iOS-26-0", "iOS 17.4", "iOS-18-0-beta"
    pub fn parse(runtime: &str) -> Option<Self> {
        // Handle various formats - normalize spaces and dots
        let normalized = runtime.replace([' ', '.'], "-");

        // Extract version numbers
        let parts: Vec<&str> = normalized.split('-').collect();

        // Find iOS prefix
        let ios_index = parts.iter().position(|&p| p.eq_ignore_ascii_case("ios"))?;

        // Must have at least major version
        if ios_index + 1 >= parts.len() {
            return None;
        }

        // Parse major version
        let major = parts[ios_index + 1].parse::<u32>().ok()?;

        // Parse minor version if available
        let minor = if ios_index + 2 < parts.len() {
            parts[ios_index + 2].parse::<u32>().unwrap_or(0)
        } else {
            0
        };

        // Parse patch version if available
        let patch = if ios_index + 3 < parts.len() {
            parts[ios_index + 3].parse::<u32>().ok()
        } else {
            None
        };

        // Check if beta
        let is_beta = normalized.to_lowercase().contains("beta") || major >= 26; // Assume iOS 26+ are beta for now

        Some(IosVersion {
            major,
            minor,
            patch,
            is_beta,
        })
    }

    /// Check if this is iOS 26 or later
    pub fn is_ios26_or_later(&self) -> bool {
        self.major >= 26
    }

    /// Get the SDK target string
    #[cfg(test)]
    pub fn sdk_target(&self) -> String {
        if self.is_beta && self.major >= 26 {
            // Use iOS 18 target for iOS 26 beta
            "arm64-apple-ios18.0-simulator".to_string()
        } else {
            format!("arm64-apple-ios{}.0-simulator", self.major)
        }
    }

    /// Get version string for display
    pub fn display_string(&self) -> String {
        let mut s = format!("iOS {}.{}", self.major, self.minor);
        if let Some(patch) = self.patch {
            s.push_str(&format!(".{patch}"));
        }
        if self.is_beta {
            s.push_str(" beta");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ios_version_parsing() {
        // Test standard format
        let v = IosVersion::parse("iOS-17-4").unwrap();
        assert_eq!(v.major, 17);
        assert_eq!(v.minor, 4);
        assert_eq!(v.patch, None);
        assert!(!v.is_beta);

        // Test iOS 26
        let v = IosVersion::parse("iOS-26-0").unwrap();
        assert_eq!(v.major, 26);
        assert_eq!(v.minor, 0);
        assert!(v.is_beta);

        // Test with beta tag
        let v = IosVersion::parse("iOS-18-0-beta").unwrap();
        assert_eq!(v.major, 18);
        assert_eq!(v.minor, 0);
        assert!(v.is_beta);

        // Test space format
        let v = IosVersion::parse("iOS 17.4").unwrap();
        assert_eq!(v.major, 17);
        assert_eq!(v.minor, 4);

        // Test SDK target
        let v = IosVersion::parse("iOS-26-0").unwrap();
        assert_eq!(v.sdk_target(), "arm64-apple-ios18.0-simulator");

        let v = IosVersion::parse("iOS-17-0").unwrap();
        assert_eq!(v.sdk_target(), "arm64-apple-ios17.0-simulator");
    }
}
