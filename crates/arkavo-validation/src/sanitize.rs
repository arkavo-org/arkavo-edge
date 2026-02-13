use serde_json::Value;

const REDACTED: &str = "[REDACTED]";
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

/// Sanitize a parsed JSON value for safe logging.
pub fn sanitize_json_value_for_log(value: &Value) -> String {
    let redacted = redact_value(value);
    let serialized = serde_json::to_string(&redacted).unwrap_or_else(|_| "{}".to_string());
    truncate_for_log(&serialized, MAX_LOG_PAYLOAD_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_redacts_sensitive_keys() {
        let input = json!({"api_key": "secret123", "name": "test"});
        let result = sanitize_json_value_for_log(&input);
        assert!(result.contains("[REDACTED]"));
        assert!(result.contains("test"));
        assert!(!result.contains("secret123"));
    }

    #[test]
    fn test_redacts_nested() {
        let input = r#"{"result":{"token":"abc","data":"ok"}}"#;
        let result = sanitize_json_line_for_log(input);
        assert!(result.contains("[REDACTED]"));
        assert!(result.contains("ok"));
    }

    #[test]
    fn test_non_json_redacted() {
        let result = sanitize_json_line_for_log("not json");
        assert!(result.contains("[non-json payload redacted]"));
    }

    #[test]
    fn test_is_sensitive_key() {
        assert!(is_sensitive_key("api_key"));
        assert!(is_sensitive_key("Authorization"));
        assert!(is_sensitive_key("x-auth-token"));
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
