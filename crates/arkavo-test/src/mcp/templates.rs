/// Embedded templates - compiled into the binary
/// This ensures templates are always consistent with the binary version

pub const ARKAVO_TEST_RUNNER_SWIFT: &str = include_str!("../../templates/XCTestRunner/ArkavoTestRunner.swift.template");
pub const ARKAVO_TEST_RUNNER_ENHANCED_SWIFT: &str = include_str!("../../templates/XCTestRunner/ArkavoTestRunnerEnhanced.swift.template");
pub const INFO_PLIST: &str = include_str!("../../templates/XCTestRunner/Info.plist.template");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_templates_are_valid() {
        // Verify templates are embedded correctly
        assert!(!ARKAVO_TEST_RUNNER_SWIFT.is_empty());
        assert!(ARKAVO_TEST_RUNNER_SWIFT.contains("struct CommandResponse"));
        
        // Verify it's the updated version with JSONValue
        assert!(!ARKAVO_TEST_RUNNER_SWIFT.contains("let result: [String: Any]?"), 
            "Template should not contain [String: Any]");
        assert!(ARKAVO_TEST_RUNNER_SWIFT.contains("enum JSONValue: Codable"), 
            "Template should contain JSONValue enum");
        assert!(ARKAVO_TEST_RUNNER_SWIFT.contains("let result: JSONValue?"),
            "Template should use JSONValue for result field");
        
        // Verify no duplicate methods
        let test_method_count = ARKAVO_TEST_RUNNER_SWIFT.matches("func testRunCommands()").count();
        assert_eq!(test_method_count, 1, 
            "Template should have exactly one testRunCommands method, found {}", test_method_count);
        
        // Verify correct method references
        assert!(ARKAVO_TEST_RUNNER_SWIFT.contains("Self.processCommand"), 
            "Template should use Self.processCommand for command handler");
        assert!(!ARKAVO_TEST_RUNNER_SWIFT.contains("Self.handleCommand"), 
            "Template should not reference non-existent handleCommand method");
        
        // Verify XCTest usage
        assert!(!ARKAVO_TEST_RUNNER_SWIFT.contains("XCTFail("), 
            "Template should not use XCTFail macro");
        assert!(!ARKAVO_TEST_RUNNER_SWIFT.contains("XCTAssertTrue("), 
            "Template should not use XCTAssertTrue macro");
    }
    
    #[test]
    fn test_info_plist_is_valid() {
        assert!(!INFO_PLIST.is_empty());
        assert!(INFO_PLIST.contains("CFBundleIdentifier"));
        assert!(INFO_PLIST.contains("CFBundleExecutable"));
        assert!(INFO_PLIST.contains("XCTestBundleIdentifier"), 
            "Info.plist should have XCTestBundleIdentifier for test bundles");
    }
}