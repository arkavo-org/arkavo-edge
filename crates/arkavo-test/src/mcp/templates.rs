/// Embedded templates - compiled into the binary
/// This ensures templates are always consistent with the binary version
pub const ARKAVO_TEST_RUNNER_SWIFT: &str =
    include_str!("../../templates/XCTestRunner/ArkavoTestRunner.swift.template");
pub const ARKAVO_TEST_RUNNER_ENHANCED_SWIFT: &str =
    include_str!("../../templates/XCTestRunner/ArkavoTestRunnerEnhanced.swift.template");
pub const ARKAVO_TEST_RUNNER_AXP_SWIFT: &str =
    include_str!("../../templates/XCTestRunner/ArkavoTestRunnerAXP.swift.template");
pub const ARKAVO_AX_BRIDGE_SWIFT: &str = include_str!("../../templates/ArkavoAXBridge.swift");
pub const INFO_PLIST: &str = include_str!("../../templates/XCTestRunner/Info.plist.template");
pub const GENERIC_AXP_HARNESS_PLIST: &str =
    include_str!("../../templates/XCTestRunner/GenericAXPHarness.plist.template");
pub const ARKAVO_AX_BRIDGE_MINIMAL_SWIFT: &str =
    include_str!("../../templates/ArkavoAXBridgeMinimal.swift");
pub const ARKAVO_TEST_RUNNER_MINIMAL_SWIFT: &str =
    include_str!("../../templates/ArkavoTestRunnerMinimal.swift");
pub const ARKAVO_AX_BRIDGE_IOS26_SWIFT: &str =
    include_str!("../../templates/ArkavoAXBridgeIOS26.swift");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn test_templates_are_valid() {
        // Verify templates are embedded correctly
        // Note: include_str! constants are never empty at compile time
        assert!(ARKAVO_TEST_RUNNER_SWIFT.contains("struct CommandResponse"));

        // Verify it's the updated version with JSONValue
        assert!(
            !ARKAVO_TEST_RUNNER_SWIFT.contains("let result: [String: Any]?"),
            "Template should not contain [String: Any]"
        );
        assert!(
            ARKAVO_TEST_RUNNER_SWIFT.contains("enum JSONValue: Codable"),
            "Template should contain JSONValue enum"
        );
        assert!(
            ARKAVO_TEST_RUNNER_SWIFT.contains("let result: JSONValue?"),
            "Template should use JSONValue for result field"
        );

        // Verify bridge architecture (no longer a test case)
        assert!(
            ARKAVO_TEST_RUNNER_SWIFT.contains("class ArkavoTestRunner: NSObject"),
            "Template should be a bridge (NSObject), not XCTestCase"
        );
        assert!(
            !ARKAVO_TEST_RUNNER_SWIFT.contains("func testRunCommands()"),
            "Template should not have testRunCommands method in bridge mode"
        );

        // Verify correct method references for bridge mode
        assert!(
            ARKAVO_TEST_RUNNER_SWIFT.contains("processCommand"),
            "Template should have processCommand for handling commands"
        );
        assert!(
            ARKAVO_TEST_RUNNER_SWIFT.contains("@objc class func setUp()"),
            "Template should have setUp method for initialization"
        );
        assert!(
            ARKAVO_TEST_RUNNER_SWIFT.contains("@objc class func initializeBridge()"),
            "Template should have initializeBridge method"
        );

        // Verify XCTest usage
        assert!(
            !ARKAVO_TEST_RUNNER_SWIFT.contains("XCTFail("),
            "Template should not use XCTFail macro"
        );
        assert!(
            !ARKAVO_TEST_RUNNER_SWIFT.contains("XCTAssertTrue("),
            "Template should not use XCTAssertTrue macro"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_info_plist_is_valid() {
        // Note: include_str! constants are never empty at compile time
        assert!(INFO_PLIST.contains("CFBundleIdentifier"));
        assert!(INFO_PLIST.contains("CFBundleExecutable"));
        assert!(
            INFO_PLIST.contains("XCTestBundleIdentifier"),
            "Info.plist should have XCTestBundleIdentifier for test bundles"
        );
    }
}
