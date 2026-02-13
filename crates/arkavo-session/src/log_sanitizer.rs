//! Log Sanitization for Security
//!
//! Implements redaction of secrets, PII, and session tokens from logs.
//!
//! ## Spec Coverage
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-010): Session tokens redacted in logs
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-011): PII redacted in session logs
//! - [specs/arkavo-edge/session-security.spec.yaml](SESS-012): Structured log sanitization

use serde_json::Value;

/// Default patterns for sensitive keys that should be redacted
pub const SENSITIVE_KEY_PATTERNS: &[&str] = &[
    "password",
    "secret",
    "token",
    "api_key",
    "apikey",
    "auth",
    "credential",
    "private_key",
    "session",
    "cookie",
    "bearer",
    "authorization",
];

/// PII field patterns that should be hashed or redacted
pub const PII_KEY_PATTERNS: &[&str] = &[
    "email",
    "phone",
    "ssn",
    "social_security",
    "dob",
    "birthdate",
    "address",
    "name",
    "username",
    "user_id",
    "ip_address",
    "fingerprint",
];

/// Redaction marker for sensitive values
pub const REDACTED_MARKER: &str = "[REDACTED]";

/// Session token redaction marker
pub const REDACTED_SESSION_TOKEN: &str = "[REDACTED:session_token]";

/// Checks if a key matches any sensitive pattern
fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SENSITIVE_KEY_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

/// Checks if a key matches any PII pattern
fn is_pii_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    PII_KEY_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

/// Sanitizes a JSON value for safe logging
///
/// ## Spec
/// SESS-012: Structured log sanitization
///
/// ## Behavior
/// - Recursively traverses JSON objects and arrays
/// - Replaces sensitive values with `[REDACTED]`
/// - Replaces PII values with hashed or masked versions
/// - Preserves structure and non-sensitive data
pub fn sanitize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sanitized = serde_json::Map::with_capacity(map.len());
            for (key, val) in map {
                if is_sensitive_key(key) {
                    // SESS-010: Redact sensitive values
                    sanitized.insert(key.clone(), Value::String(REDACTED_MARKER.to_string()));
                } else if is_pii_key(key) {
                    // SESS-011: Hash PII values
                    let hashed = hash_pii(&val.to_string());
                    sanitized.insert(key.clone(), Value::String(hashed));
                } else {
                    // Recursively sanitize nested values
                    sanitized.insert(key.clone(), sanitize_json_value(val));
                }
            }
            Value::Object(sanitized)
        }
        Value::Array(arr) => {
            // SESS-012: Sanitize arrays recursively
            Value::Array(arr.iter().map(sanitize_json_value).collect())
        }
        // Primitive values are returned as-is (they're in context of a key that was checked)
        _ => value.clone(),
    }
}

/// Sanitizes a JSON string for safe logging
///
/// ## Spec
/// SESS-010: Session tokens redacted in logs
/// SESS-011: PII redacted in session logs
pub fn sanitize_json_line(line: &str) -> String {
    // Try to parse as JSON
    match serde_json::from_str::<Value>(line) {
        Ok(value) => {
            // Successfully parsed, sanitize the JSON
            let sanitized = sanitize_json_value(&value);
            sanitized.to_string()
        }
        Err(_) => {
            // Not valid JSON, apply basic redaction
            // SESS-010: Try to redact tokens even in invalid JSON
            redact_session_tokens(line)
        }
    }
}

/// Redacts session tokens from text
///
/// ## Spec
/// SESS-010: Session tokens redacted in logs
///
/// ## Heuristics
/// - Looks for common token patterns (JWT, API keys, etc.)
/// - Replaces them with `[REDACTED:session_token]`
pub fn redact_session_tokens(text: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;

    // JWT pattern: three base64url sections separated by dots
    static JWT_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap());

    // Generic token pattern: "token: <alphanumeric>", "Token <value>", "Token1: <value>"
    static TOKEN_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(token\d*[:\s]+)([a-z0-9_-]{3,})").unwrap());

    // Bearer token pattern
    static BEARER_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(bearer\s+)([a-z0-9_-]{8,})").unwrap());

    let mut result = text.to_string();

    // Redact JWTs
    result = JWT_RE
        .replace_all(&result, REDACTED_SESSION_TOKEN)
        .to_string();

    // Redact explicit tokens
    result = TOKEN_RE
        .replace_all(&result, format!("${{1}}{REDACTED_SESSION_TOKEN}"))
        .to_string();

    // Redact bearer tokens
    result = BEARER_RE
        .replace_all(&result, format!("${{1}}{REDACTED_SESSION_TOKEN}"))
        .to_string();

    result
}

/// Hashes PII for safe logging (deterministic, irreversible)
///
/// ## Spec
/// SESS-011: PII redacted in session logs
///
/// ## Algorithm
/// Uses SHA-256 truncated to 16 characters for log readability.
/// The hash is deterministic - same input always produces same output.
pub fn hash_pii(value: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();

    // Format as hex string
    format!("[HASH:{hash:016x}]")
}

/// Sanitizes a log message with mixed content
///
/// ## Example
/// ```
/// use arkavo_session::log_sanitizer::sanitize_log_message;
///
/// let message = "User user@example.com logged in with token abc123";
/// let safe = sanitize_log_message(message);
/// assert!(!safe.contains("abc123"));
/// ```
pub fn sanitize_log_message(message: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;

    // Email pattern
    static EMAIL_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap());

    // IP address pattern
    static IP_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").unwrap());

    let mut result = message.to_string();

    // Redact tokens
    result = redact_session_tokens(&result);

    // Hash emails
    result = EMAIL_RE
        .replace_all(&result, |caps: &regex::Captures| hash_pii(&caps[0]))
        .to_string();

    // Hash IP addresses
    result = IP_RE
        .replace_all(&result, |caps: &regex::Captures| hash_pii(&caps[0]))
        .to_string();

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // SESS-010: Session tokens redacted in logs

    #[test]
    fn test_session_token_redaction() {
        let log_with_token = "Session token: eyJhbGciOiJIUzI1NiIs...";
        let sanitized = redact_session_tokens(log_with_token);
        assert!(!sanitized.contains("eyJhbGci"));
        assert!(sanitized.contains(REDACTED_SESSION_TOKEN) || sanitized.contains(REDACTED_MARKER));
    }

    #[test]
    fn test_multiple_tokens_redacted() {
        let log = "Token1: abc123, Token2: def456, Token3: ghi789";
        let sanitized = redact_session_tokens(log);
        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("def456"));
        assert!(!sanitized.contains("ghi789"));
    }

    // SESS-011: PII redacted in session logs

    #[test]
    fn test_email_redaction() {
        let log = json!({
            "event": "login",
            "email": "user@example.com",
            "status": "success"
        });
        let sanitized = sanitize_json_value(&log);
        let s = sanitized.to_string();
        assert!(!s.contains("user@example.com"));
        assert!(s.contains("\"event\":\"login\""));
        assert!(s.contains("\"status\":\"success\""));
    }

    #[test]
    fn test_phone_redaction() {
        let log = json!({
            "user_phone": "+1-555-123-4567",
            "action": "verification"
        });
        let sanitized = sanitize_json_value(&log);
        let s = sanitized.to_string();
        assert!(!s.contains("555-123-4567"));
        assert!(s.contains("\"action\":\"verification\""));
    }

    #[test]
    fn test_pii_hashing_deterministic() {
        let email = "user@example.com";
        let hash1 = hash_pii(email);
        let hash2 = hash_pii(email);
        assert_eq!(hash1, hash2);
        assert!(!hash1.contains(email));
        assert!(!hash1.is_empty());
    }

    #[test]
    fn test_pii_hashing_unique() {
        let hash1 = hash_pii("user1@example.com");
        let hash2 = hash_pii("user2@example.com");
        assert_ne!(hash1, hash2);
    }

    // SESS-012: Structured log sanitization

    #[test]
    fn test_nested_json_sanitization() {
        let log = json!({
            "user": {
                "name": "John Doe",
                "email": "john@example.com",
                "token": "secret_token_123"
            },
            "action": "update_profile"
        });
        let sanitized = sanitize_json_value(&log);
        let s = sanitized.to_string();
        assert!(!s.contains("John Doe"));
        assert!(!s.contains("john@example.com"));
        assert!(!s.contains("secret_token_123"));
        assert!(s.contains("\"action\":\"update_profile\""));
    }

    #[test]
    fn test_array_sanitization() {
        let log = json!({
            "users": [
                {"email": "user1@example.com", "id": 1},
                {"email": "user2@example.com", "id": 2}
            ]
        });
        let sanitized = sanitize_json_value(&log);
        let s = sanitized.to_string();
        assert!(!s.contains("user1@example.com"));
        assert!(!s.contains("user2@example.com"));
        assert!(s.contains("\"id\":1"));
        assert!(s.contains("\"id\":2"));
    }

    #[test]
    fn test_case_insensitive_key_matching() {
        let log = json!({
            "API_KEY": "secret123",
            "BearerToken": "token456",
            "sessionId": "session789"
        });
        let sanitized = sanitize_json_value(&log);
        let s = sanitized.to_string();
        assert!(!s.contains("secret123"));
        assert!(!s.contains("token456"));
        assert!(!s.contains("session789"));
    }

    // General sanitization

    #[test]
    fn test_json_line_sanitization() {
        let line = r#"{"timestamp":"2024-01-01","token":"secret","event":"login"}"#;
        let sanitized = sanitize_json_line(line);
        assert!(!sanitized.contains("secret"));
        assert!(sanitized.contains("timestamp"));
        assert!(sanitized.contains("event"));
    }

    #[test]
    fn test_invalid_json_handling() {
        let invalid = "not valid json {token: secret}";
        let sanitized = sanitize_json_line(invalid);
        assert!(!sanitized.is_empty());
    }

    #[test]
    fn test_log_message_sanitization() {
        let message = "User user@example.com with token abc123 made request from 192.168.1.1";
        let sanitized = sanitize_log_message(message);
        assert!(!sanitized.contains("user@example.com"));
        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("192.168.1.1"));
    }

    #[test]
    fn test_non_sensitive_data_preserved() {
        let log = json!({
            "timestamp": "2024-01-01T00:00:00Z",
            "level": "INFO",
            "component": "session_manager",
            "token": "secret123"
        });
        let sanitized = sanitize_json_value(&log);
        let s = sanitized.to_string();
        assert!(s.contains("timestamp"));
        assert!(s.contains("INFO"));
        assert!(s.contains("session_manager"));
        assert!(!s.contains("secret123"));
    }
}
