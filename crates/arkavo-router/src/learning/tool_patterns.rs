//! Tool call format definitions for agent swarms
//!
//! Defines the different formats LLMs use to invoke tools.
//! The actual learning uses generic Lesson/LessonPattern types with metadata.

use serde::{Deserialize, Serialize};

/// Tool call format types - how LLMs format tool invocations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolCallFormat {
    /// Triple-backtick fence with language tag: ```tool_name\n{json}```
    Fence,
    /// XML-style: <tool_name>...</tool_name>
    Xml,
    /// Raw JSON: {"name": "tool", "parameters": {...}}
    Json,
    /// Python function call: tool_name(param=value)
    PythonCall,
}

impl std::fmt::Display for ToolCallFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fence => write!(f, "fence"),
            Self::Xml => write!(f, "xml"),
            Self::Json => write!(f, "json"),
            Self::PythonCall => write!(f, "python"),
        }
    }
}

impl std::str::FromStr for ToolCallFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "fence" => Ok(Self::Fence),
            "xml" => Ok(Self::Xml),
            "json" => Ok(Self::Json),
            "python" | "pythoncall" => Ok(Self::PythonCall),
            _ => Err(format!("Unknown tool call format: {s}")),
        }
    }
}

impl ToolCallFormat {
    /// Generate a few-shot example for a tool call
    pub fn format_example(&self, tool_name: &str, args: &serde_json::Value) -> String {
        match self {
            Self::Fence => {
                format!(
                    "Example of calling {}:\n```{}\n{}\n```",
                    tool_name,
                    tool_name,
                    serde_json::to_string_pretty(args).unwrap_or_default()
                )
            }
            Self::Xml => {
                format!(
                    "Example of calling {}:\n<{}>\n{}\n</{}>",
                    tool_name,
                    tool_name,
                    serde_json::to_string_pretty(args).unwrap_or_default(),
                    tool_name
                )
            }
            Self::Json => {
                format!(
                    "Example of calling {}:\n{}",
                    tool_name,
                    serde_json::to_string_pretty(&serde_json::json!({
                        "name": tool_name,
                        "parameters": args
                    }))
                    .unwrap_or_default()
                )
            }
            Self::PythonCall => {
                let params = if let Some(obj) = args.as_object() {
                    obj.iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    String::new()
                };
                format!("Example of calling {tool_name}:\n{tool_name}({params})")
            }
        }
    }
}

/// Sanitize arguments by removing potential secrets
pub fn sanitize_args(args: &serde_json::Value) -> serde_json::Value {
    match args {
        serde_json::Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in map {
                let key_lower = key.to_lowercase();
                if key_lower.contains("key")
                    || key_lower.contains("token")
                    || key_lower.contains("password")
                    || key_lower.contains("secret")
                {
                    sanitized.insert(key.clone(), serde_json::json!("<REDACTED>"));
                } else {
                    sanitized.insert(key.clone(), sanitize_args(value));
                }
            }
            serde_json::Value::Object(sanitized)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(sanitize_args).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_call_format_display() {
        assert_eq!(ToolCallFormat::Fence.to_string(), "fence");
        assert_eq!(ToolCallFormat::Xml.to_string(), "xml");
        assert_eq!(ToolCallFormat::Json.to_string(), "json");
        assert_eq!(ToolCallFormat::PythonCall.to_string(), "python");
    }

    #[test]
    fn test_tool_call_format_parse() {
        assert_eq!(
            "fence".parse::<ToolCallFormat>().unwrap(),
            ToolCallFormat::Fence
        );
        assert_eq!(
            "XML".parse::<ToolCallFormat>().unwrap(),
            ToolCallFormat::Xml
        );
        assert!("invalid".parse::<ToolCallFormat>().is_err());
    }

    #[test]
    fn test_format_example_fence() {
        let args = serde_json::json!({"id": 4});
        let example = ToolCallFormat::Fence.format_example("get_sector", &args);
        assert!(example.contains("```get_sector"));
        assert!(example.contains("\"id\": 4"));
    }

    #[test]
    fn test_sanitize_args() {
        let args = serde_json::json!({
            "sector_id": 4,
            "api_key": "sk-secret-key-12345",
            "user": "test"
        });

        let sanitized = sanitize_args(&args);
        assert_eq!(sanitized["api_key"], "<REDACTED>");
        assert_eq!(sanitized["sector_id"], 4);
        assert_eq!(sanitized["user"], "test");
    }
}
