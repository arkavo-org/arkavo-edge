use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::TestError;

/// Validates test names to prevent command injection
pub fn validate_test_name(test_name: &str) -> Result<(), TestError> {
    // Check length
    if test_name.is_empty() || test_name.len() > 256 {
        return Err(TestError::Validation(
            "Test name must be between 1 and 256 characters".to_string(),
        ));
    }

    // Allow alphanumeric, underscores, colons, dots, and hyphens
    // This covers common test naming patterns like:
    // - test_module::test_function (Rust)
    // - TestClass.testMethod (Swift)
    // - describe.it (JavaScript)
    // - test_function (Python)
    // - TestFunction (Go)
    if !test_name.chars().all(|c| {
        c.is_alphanumeric() || c == '_' || c == ':' || c == '.' || c == '-' || c == '/'
    }) {
        return Err(TestError::Validation(format!(
            "Invalid test name '{}'. Only alphanumeric characters, underscores, colons, dots, hyphens, and forward slashes are allowed",
            test_name
        )));
    }

    // Prevent directory traversal
    if test_name.contains("..") {
        return Err(TestError::Validation(
            "Test name cannot contain directory traversal patterns".to_string(),
        ));
    }

    // Prevent absolute paths
    if test_name.starts_with('/') || test_name.starts_with('\\') {
        return Err(TestError::Validation(
            "Test name cannot be an absolute path".to_string(),
        ));
    }

    Ok(())
}

/// Validates file paths to prevent directory traversal attacks
pub fn validate_path(path: &Path, allowed_base: &Path) -> Result<PathBuf, TestError> {
    // Resolve to absolute paths
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        allowed_base
            .join(path)
            .canonicalize()
            .map_err(|e| TestError::Validation(format!("Invalid path: {}", e)))?
    };

    let base_absolute = allowed_base
        .canonicalize()
        .map_err(|e| TestError::Validation(format!("Invalid base path: {}", e)))?;

    // Check if the resolved path is within the allowed directory
    if !absolute_path.starts_with(&base_absolute) {
        return Err(TestError::Validation(format!(
            "Path '{}' is outside the allowed directory",
            path.display()
        )));
    }

    Ok(absolute_path)
}

/// Default timeout for test execution
pub const DEFAULT_TEST_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

/// Maximum allowed timeout
pub const MAX_TEST_TIMEOUT: Duration = Duration::from_secs(3600); // 1 hour

/// Validates and normalizes timeout duration
pub fn validate_timeout(timeout_secs: Option<u64>) -> Duration {
    match timeout_secs {
        Some(secs) => {
            let duration = Duration::from_secs(secs);
            if duration > MAX_TEST_TIMEOUT {
                MAX_TEST_TIMEOUT
            } else if duration.is_zero() {
                DEFAULT_TEST_TIMEOUT
            } else {
                duration
            }
        }
        None => DEFAULT_TEST_TIMEOUT,
    }
}

/// List of allowed test commands
pub const ALLOWED_TEST_COMMANDS: &[&str] = &[
    "cargo",
    "swift",
    "xcodebuild",
    "npm",
    "yarn",
    "jest",
    "mocha",
    "pytest",
    "python",
    "go",
];

/// Validates that a command is in the allowed list
pub fn validate_command(command: &str) -> Result<(), TestError> {
    if !ALLOWED_TEST_COMMANDS.contains(&command) {
        return Err(TestError::Validation(format!(
            "Command '{}' is not in the allowed list. Allowed commands: {:?}",
            command, ALLOWED_TEST_COMMANDS
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_test_name_valid() {
        assert!(validate_test_name("test_function").is_ok());
        assert!(validate_test_name("module::test_function").is_ok());
        assert!(validate_test_name("TestClass.testMethod").is_ok());
        assert!(validate_test_name("test-suite/test-case").is_ok());
        assert!(validate_test_name("test_123").is_ok());
    }

    #[test]
    fn test_validate_test_name_invalid() {
        assert!(validate_test_name("").is_err());
        assert!(validate_test_name("test;rm -rf /").is_err());
        assert!(validate_test_name("test && echo pwned").is_err());
        assert!(validate_test_name("test | cat /etc/passwd").is_err());
        assert!(validate_test_name("../../../etc/passwd").is_err());
        assert!(validate_test_name("/absolute/path").is_err());
        assert!(validate_test_name("test\nname").is_err());
        assert!(validate_test_name("test$name").is_err());
        assert!(validate_test_name("test`name`").is_err());
    }

    #[test]
    fn test_validate_test_name_length() {
        let long_name = "a".repeat(257);
        assert!(validate_test_name(&long_name).is_err());
    }

    #[test]
    fn test_validate_timeout() {
        assert_eq!(validate_timeout(None), DEFAULT_TEST_TIMEOUT);
        assert_eq!(validate_timeout(Some(0)), DEFAULT_TEST_TIMEOUT);
        assert_eq!(validate_timeout(Some(60)), Duration::from_secs(60));
        assert_eq!(validate_timeout(Some(7200)), MAX_TEST_TIMEOUT);
    }

    #[test]
    fn test_validate_command() {
        assert!(validate_command("cargo").is_ok());
        assert!(validate_command("npm").is_ok());
        assert!(validate_command("rm").is_err());
        assert!(validate_command("curl").is_err());
    }
}