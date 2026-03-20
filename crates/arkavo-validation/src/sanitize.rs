use serde_json::Value;

const REDACTED: &str = "[REDACTED]";
/// Sentinel value for round-trip preservation in UI redaction.
pub const REDACTED_SENTINEL: &str = "__ARKAVO_REDACTED__";
const MAX_LOG_PAYLOAD_CHARS: usize = 2048;
const SENSITIVE_KEY_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "token",
    "secret",
    "password",
    "authorization",
    "auth",
    "cookie",
    "session",
    "private_key",
    "bearer",
    "credential",
    "access_key",
    "secret_key",
];

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SENSITIVE_KEY_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = serde_json::Map::with_capacity(map.len());
            for (key, val) in map {
                if is_sensitive_key(key) {
                    redacted.insert(key.clone(), Value::String(REDACTED.to_string()));
                } else {
                    redacted.insert(key.clone(), redact_value(val));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        _ => value.clone(),
    }
}

fn truncate_for_log(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}... [truncated, {count} chars total]")
}

/// Sanitize a JSON line for safe logging: redacts sensitive fields and truncates.
pub fn sanitize_json_line_for_log(line: &str) -> String {
    match serde_json::from_str::<Value>(line) {
        Ok(value) => sanitize_json_value_for_log(&value),
        Err(_) => truncate_for_log("[non-json payload redacted]", MAX_LOG_PAYLOAD_CHARS),
    }
}

/// Check whether a key is sensitive (public for reuse).
pub fn is_sensitive(key: &str) -> bool {
    is_sensitive_key(key)
}

/// Redact sensitive values in a JSON object for UI display.
///
/// Uses `REDACTED_SENTINEL` instead of `[REDACTED]` so the UI can detect
/// and preserve redacted values on round-trip (e.g., config saves).
pub fn redact_for_ui(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = serde_json::Map::with_capacity(map.len());
            for (key, val) in map {
                if is_sensitive_key(key) {
                    redacted.insert(key.clone(), Value::String(REDACTED_SENTINEL.to_string()));
                } else {
                    redacted.insert(key.clone(), redact_for_ui(val));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_for_ui).collect()),
        _ => value.clone(),
    }
}

/// Check if a value is the redacted sentinel (for round-trip preservation).
pub fn is_redacted_sentinel(value: &str) -> bool {
    value == REDACTED_SENTINEL
}

/// Sanitize a parsed JSON value for safe logging.
pub fn sanitize_json_value_for_log(value: &Value) -> String {
    let redacted = redact_value(value);
    let serialized = serde_json::to_string(&redacted).unwrap_or_else(|_| "{}".to_string());
    truncate_for_log(&serialized, MAX_LOG_PAYLOAD_CHARS)
}

#[cfg(test)]
mod tests {
    //! Unit tests for log sanitization and PII redaction.
    //!
    //! ## Spec Coverage
    //! - [specs/arkavo-edge/network-security.spec.yaml](NET-017): Egress audit logging with redaction
    //! - [specs/arkavo-edge/tdf-security.spec.yaml](TDFS-002): Configuration bundle TDF encryption - secrets handling
    //!
    //! ## Security Principle
    //! Sensitive data must never appear in logs, even in encrypted/debug contexts.

    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    #[spec("VAL-003")]
    #[test]
    fn test_redacts_sensitive_keys() {
        let input = json!({"api_key": "secret123", "name": "test"});
        let result = sanitize_json_value_for_log(&input);
        assert!(result.contains("[REDACTED]"));
        assert!(result.contains("test"));
        assert!(!result.contains("secret123"));
    }

    #[spec("VAL-003")]
    #[test]
    fn test_redacts_nested() {
        let input = r#"{"result":{"token":"abc","data":"ok"}}"#;
        let result = sanitize_json_line_for_log(input);
        assert!(result.contains("[REDACTED]"));
        assert!(result.contains("ok"));
    }

    #[spec("VAL-003")]
    #[test]
    fn test_non_json_redacted() {
        let result = sanitize_json_line_for_log("not json");
        assert!(result.contains("[non-json payload redacted]"));
    }

    #[spec("VAL-003")]
    #[test]
    fn test_is_sensitive_key() {
        assert!(is_sensitive_key("api_key"));
        assert!(is_sensitive_key("Authorization"));
        assert!(is_sensitive_key("x-auth-token"));
        assert!(is_sensitive_key("aws_credential"));
        assert!(is_sensitive_key("access_key_id"));
        assert!(is_sensitive_key("secret_key"));
        assert!(!is_sensitive_key("name"));
        assert!(!is_sensitive_key("data"));
    }

    #[test]
    fn test_truncation() {
        let long_str = "x".repeat(3000);
        let result = truncate_for_log(&long_str, MAX_LOG_PAYLOAD_CHARS);
        assert!(result.len() < 3000);
        assert!(result.contains("truncated"));
    }
}
