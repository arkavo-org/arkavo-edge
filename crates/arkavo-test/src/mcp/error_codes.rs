use serde::{Deserialize, Serialize};

/// Standard MCP error codes for consistent error handling
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ErrorCode(pub i32);

impl ErrorCode {
    // Standard JSON-RPC error codes
    pub const PARSE_ERROR: Self = Self(-32700);
    pub const INVALID_REQUEST: Self = Self(-32600);
    pub const METHOD_NOT_FOUND: Self = Self(-32601);
    pub const INVALID_PARAMS: Self = Self(-32602);
    pub const INTERNAL_ERROR: Self = Self(-32603);

    // Custom MCP error codes (range -32000 to -32099)
    pub const TOOL_NOT_FOUND: Self = Self(-32000);
    pub const TOOL_EXECUTION_FAILED: Self = Self(-32001);
    pub const INVALID_TOOL_PARAMS: Self = Self(-32002);
    pub const VALIDATION_ERROR: Self = Self(-32003);
    pub const TIMEOUT_ERROR: Self = Self(-32004);
    pub const STATE_ERROR: Self = Self(-32005);
    pub const PROJECT_TYPE_UNKNOWN: Self = Self(-32006);
    pub const TEST_NOT_FOUND: Self = Self(-32007);
    pub const SECURITY_VIOLATION: Self = Self(-32008);
    pub const PERSISTENCE_ERROR: Self = Self(-32009);
    pub const RESOURCE_NOT_FOUND: Self = Self(-32010);
    pub const PERMISSION_DENIED: Self = Self(-32011);
    pub const QUOTA_EXCEEDED: Self = Self(-32012);
}

impl ErrorCode {
    /// Get a human-readable description of the error code
    pub fn description(&self) -> &'static str {
        match self.0 {
            -32700 => "Parse error: Invalid JSON was received",
            -32600 => "Invalid Request: The JSON sent is not a valid Request object",
            -32601 => "Method not found: The method does not exist or is not available",
            -32602 => "Invalid params: Invalid method parameter(s)",
            -32603 => "Internal error: Internal JSON-RPC error",
            -32000 => "Tool not found: The requested tool does not exist",
            -32001 => "Tool execution failed: The tool encountered an error during execution",
            -32002 => "Invalid tool params: The tool parameters are invalid or missing",
            -32003 => "Validation error: Input validation failed",
            -32004 => "Timeout error: Operation timed out",
            -32005 => "State error: State management operation failed",
            -32006 => "Project type unknown: Unable to detect project type",
            -32007 => "Test not found: The specified test could not be found",
            -32008 => "Security violation: Operation blocked for security reasons",
            -32009 => "Persistence error: Failed to save or load persistent data",
            -32010 => "Resource not found: The requested resource does not exist",
            -32011 => "Permission denied: Insufficient permissions for this operation",
            -32012 => "Quota exceeded: Operation would exceed configured limits",
            _ => "Unknown error",
        }
    }
}

/// Structured error response for MCP protocol
#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ErrorResponse {
    /// Create a new error response
    pub fn new(code: ErrorCode, message: String) -> Self {
        Self {
            code,
            message,
            data: None,
        }
    }

    /// Create an error response with additional data
    pub fn with_data(code: ErrorCode, message: String, data: serde_json::Value) -> Self {
        Self {
            code,
            message,
            data: Some(data),
        }
    }

    /// Create an error response using the default description
    pub fn from_code(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.description().to_string(),
            data: None,
        }
    }
}

/// Convert TestError to ErrorResponse
impl From<crate::TestError> for ErrorResponse {
    fn from(error: crate::TestError) -> Self {
        match &error {
            crate::TestError::Mcp(msg) => ErrorResponse::new(
                ErrorCode::INTERNAL_ERROR,
                format!("MCP error: {}", msg),
            ),
            crate::TestError::Validation(msg) => ErrorResponse::new(
                ErrorCode::VALIDATION_ERROR,
                format!("Validation error: {}", msg),
            ),
            crate::TestError::Execution(msg) => ErrorResponse::new(
                ErrorCode::TOOL_EXECUTION_FAILED,
                format!("Execution error: {}", msg),
            ),
            crate::TestError::Io(err) => ErrorResponse::new(
                ErrorCode::INTERNAL_ERROR,
                format!("IO error: {}", err),
            ),
            _ => ErrorResponse::new(
                ErrorCode::INTERNAL_ERROR,
                error.to_string(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_descriptions() {
        assert_eq!(ErrorCode::PARSE_ERROR.description(), "Parse error: Invalid JSON was received");
        assert_eq!(ErrorCode::TOOL_NOT_FOUND.description(), "Tool not found: The requested tool does not exist");
        assert_eq!(ErrorCode::SECURITY_VIOLATION.description(), "Security violation: Operation blocked for security reasons");
    }

    #[test]
    fn test_error_response_creation() {
        let err = ErrorResponse::new(ErrorCode::VALIDATION_ERROR, "Invalid test name".to_string());
        assert_eq!(err.code, ErrorCode::VALIDATION_ERROR);
        assert_eq!(err.message, "Invalid test name");
        assert!(err.data.is_none());
    }

    #[test]
    fn test_error_response_with_data() {
        let data = serde_json::json!({"field": "test_name", "value": "rm -rf /"});
        let err = ErrorResponse::with_data(
            ErrorCode::SECURITY_VIOLATION,
            "Dangerous command detected".to_string(),
            data.clone(),
        );
        assert_eq!(err.code, ErrorCode::SECURITY_VIOLATION);
        assert_eq!(err.data, Some(data));
    }

    #[test]
    fn test_error_conversion() {
        let test_err = crate::TestError::Validation("Bad input".to_string());
        let err_resp: ErrorResponse = test_err.into();
        assert_eq!(err_resp.code, ErrorCode::VALIDATION_ERROR);
        assert!(err_resp.message.contains("Bad input"));
    }
}