#[cfg(test)]
mod tests {
    use arkavo_test::mcp::xcode_version::XcodeVersion;

    #[test]
    fn test_xcode_version_detection() {
        // This test will only work if Xcode is installed
        match XcodeVersion::detect() {
            Ok(version) => {
                println!(
                    "Detected Xcode version: {}.{}.{}",
                    version.major, version.minor, version.patch
                );

                // Check supported features
                println!("Supported features:");
                println!("  - Boot status: {}", version.supports_bootstatus());
                println!("  - Privacy: {}", version.supports_privacy());
                println!("  - UI commands: {}", version.supports_ui_commands());
                println!(
                    "  - Device appearance: {}",
                    version.supports_device_appearance()
                );
                println!(
                    "  - Push notifications: {}",
                    version.supports_push_notification()
                );
                println!("  - Clone: {}", version.supports_clone());
                println!("  - Device pair: {}", version.supports_device_pair());
                println!("  - Device focus: {}", version.supports_device_focus());
                println!(
                    "  - Device streaming: {}",
                    version.supports_device_streaming()
                );
                println!(
                    "  - Enhanced UI interaction: {}",
                    version.supports_enhanced_ui_interaction()
                );

                // Test version comparisons
                assert!(version >= XcodeVersion::new(10, 0, 0));
            }
            Err(e) => {
                println!("Could not detect Xcode version: {}", e);
                // This is not a failure if Xcode is not installed
            }
        }
    }

    #[test]
    fn test_version_comparison() {
        let v1 = XcodeVersion::new(15, 0, 0);
        let v2 = XcodeVersion::new(16, 0, 0);
        let v3 = XcodeVersion::new(15, 1, 0);
        let v4 = XcodeVersion::new(15, 0, 1);

        assert!(v1 < v2);
        assert!(v1 < v3);
        assert!(v1 < v4);
        assert!(v2 > v1);
        assert!(v3 > v1);
        assert!(v4 > v1);
    }

    #[test]
    fn test_feature_support() {
        let xcode11 = XcodeVersion::new(11, 0, 0);
        assert!(xcode11.supports_bootstatus());
        assert!(!xcode11.supports_privacy());
        assert!(!xcode11.supports_ui_commands());

        let xcode15 = XcodeVersion::new(15, 0, 0);
        assert!(xcode15.supports_bootstatus());
        assert!(xcode15.supports_privacy());
        assert!(xcode15.supports_ui_commands());
        assert!(!xcode15.supports_enhanced_ui_interaction());

        let xcode26 = XcodeVersion::new(26, 0, 0);
        assert!(xcode26.supports_bootstatus());
        assert!(xcode26.supports_privacy());
        assert!(xcode26.supports_ui_commands());
        assert!(xcode26.supports_enhanced_ui_interaction());
    }
}
