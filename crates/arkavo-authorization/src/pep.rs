//! MCP JSON-RPC method classification for the COAZ-MCP CWT profile (PR 4, no CEL).

use crate::error::{AuthorizationError, Result};
use serde_json::Value;

pub fn is_pass_through(method: &str) -> bool {
    method == "ping" || method.starts_with("notifications/")
}

pub fn is_hardcoded_mapped(method: &str) -> bool {
    method == "tools/call" || method == "tools/list"
}

pub fn tool_name_from_params(params: Option<&Value>) -> Result<&str> {
    params
        .and_then(|p| p.get("name").or_else(|| p.get("tool_name")))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AuthorizationError::Mapping("tools/call requires params.name".into()))
}

/// Subject CWT from an explicit Bearer, else the CWT-only env var.
/// `ANTHROPIC_API_KEY` is never accepted.
pub fn subject_cwt_from(explicit: Option<&str>) -> Option<String> {
    if let Some(t) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let t = t
            .strip_prefix("Bearer ")
            .or_else(|| t.strip_prefix("bearer "))
            .unwrap_or(t);
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    std::env::var("CLAUDE_CODE_SESSION_ACCESS_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pass_through_and_unknown() {
        assert!(is_pass_through("ping"));
        assert!(is_pass_through("notifications/initialized"));
        assert!(!is_pass_through("initialize"));
        assert!(!is_pass_through("resources/list"));
        assert!(is_hardcoded_mapped("tools/call"));
        assert!(is_hardcoded_mapped("tools/list"));
        assert!(!is_hardcoded_mapped("prompts/list"));
    }

    #[test]
    fn tools_call_name() {
        assert_eq!(
            tool_name_from_params(Some(&json!({"name": "git_commit"}))).unwrap(),
            "git_commit"
        );
        assert!(tool_name_from_params(Some(&json!({}))).is_err());
        assert_eq!(
            tool_name_from_params(Some(&json!({"tool_name": "echo"}))).unwrap(),
            "echo"
        );
    }

    #[test]
    fn subject_cwt_strips_bearer_and_ignores_empty() {
        assert_eq!(
            subject_cwt_from(Some("Bearer abc")),
            Some("abc".to_string())
        );
        assert_eq!(subject_cwt_from(Some("   ")), None);
    }
}
