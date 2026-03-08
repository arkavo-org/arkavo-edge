use arkavo_mcp_tools::registry::ToolInfo;
use serde_json::Value;
use std::fmt::Write as _;

/// Generate a realistic example value for a parameter based on its name and JSON schema.
pub fn example_value_for_param(name: &str, schema: &Value) -> String {
    // Check for enum values first
    if let Some(enum_vals) = schema.get("enum").and_then(|e| e.as_array())
        && let Some(first) = enum_vals.first().and_then(|v| v.as_str())
    {
        return first.to_string();
    }

    // Check for default value
    if let Some(default) = schema.get("default") {
        return match default {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => format!("{default}"),
        };
    }

    let param_type = schema
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("string");

    // Type-aware generation, refined by parameter name patterns
    match param_type {
        "boolean" => "true".to_string(),
        "integer" => match name {
            "x" | "y" | "z" => "10".to_string(),
            "limit" | "count" | "max" | "top_k" => "5".to_string(),
            "port" => "8080".to_string(),
            "timeout" | "timeout_ms" => "30000".to_string(),
            _ => "1".to_string(),
        },
        "number" => match name {
            "temperature" => "0.7".to_string(),
            "top_p" => "0.9".to_string(),
            "threshold" | "score" => "0.5".to_string(),
            _ => "1.0".to_string(),
        },
        "array" => {
            if let Some(items) = schema.get("items") {
                let item_example = example_value_for_param("item", items);
                format!("[{item_example}]")
            } else {
                "[\"item1\", \"item2\"]".to_string()
            }
        }
        "object" => example_object_from_schema(name, schema),
        // Default: string — use name-based heuristics
        _ => example_string_for_name(name),
    }
}

/// Generate an example JSON object using actual property names from the schema.
/// Falls back to `{"key": "value"}` when no properties are defined.
fn example_object_from_schema(_name: &str, schema: &Value) -> String {
    let props = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) if !p.is_empty() => p,
        _ => return "{\"key\": \"value\"}".to_string(),
    };

    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Show required props first, then up to 1 optional
    let mut pairs = Vec::new();
    for req in &required {
        if let Some(prop_schema) = props.get(*req) {
            let val = example_value_for_param(req, prop_schema);
            pairs.push(format!("\"{req}\": \"{val}\""));
        }
    }
    for (key, prop_schema) in props {
        if !required.contains(&key.as_str()) && pairs.len() < required.len() + 1 {
            let val = example_value_for_param(key, prop_schema);
            pairs.push(format!("\"{key}\": \"{val}\""));
        }
    }

    if pairs.is_empty() {
        "{\"key\": \"value\"}".to_string()
    } else {
        format!("{{{}}}", pairs.join(", "))
    }
}

fn example_string_for_name(name: &str) -> String {
    let n = name.to_lowercase();
    if n.contains("path") || n.contains("file") || n.contains("dir") {
        "/tmp/example.txt".to_string()
    } else if n.contains("url") || n.contains("endpoint") || n.contains("address") {
        "https://example.com".to_string()
    } else if n.contains("query") || n.contains("search") || n.contains("pattern") {
        "search term".to_string()
    } else if n.contains("command") || n.contains("cmd") {
        "ls -la".to_string()
    } else if n.contains("message") || n.contains("text") || n.contains("content") {
        "Hello world".to_string()
    } else if n.contains("name") || n.contains("label") || n.contains("title") {
        "my-example".to_string()
    } else if n.contains("id") {
        "abc123".to_string()
    } else if n.contains("location") || n.contains("city") {
        "Tokyo".to_string()
    } else if n.contains("lang") || n.contains("language") {
        "en".to_string()
    } else if n.contains("format") || n.contains("type") {
        "json".to_string()
    } else if n.contains("key") || n.contains("token") {
        "my-key".to_string()
    } else {
        "example_value".to_string()
    }
}

/// Generate a complete fence-format example for a tool, using all required parameters.
pub fn generate_fence_example(tool: &ToolInfo) -> String {
    let mut result = format!("```{}\n", tool.name);

    let required: Vec<&str> = tool
        .schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if let Some(props) = tool.schema.get("properties").and_then(|p| p.as_object()) {
        // Show required params first, then optional (up to 2)
        let mut shown = 0;
        for name in &required {
            if let Some(prop_schema) = props.get(*name) {
                let val = example_value_for_param(name, prop_schema);
                let _ = writeln!(result, "{name}: {val}");
                shown += 1;
            }
        }
        // Add up to 2 optional params so the model sees they exist
        for (name, prop_schema) in props {
            if !required.contains(&name.as_str()) && shown < required.len() + 2 {
                let val = example_value_for_param(name, prop_schema);
                let _ = writeln!(result, "{name}: {val}");
                shown += 1;
            }
        }
        if shown == 0 {
            // No-param tool — still show the fence
        }
    }

    result.push_str("```");
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_example_value_enum() {
        let schema = json!({"type": "string", "enum": ["celsius", "fahrenheit"]});
        assert_eq!(example_value_for_param("unit", &schema), "celsius");
    }

    #[test]
    fn test_example_value_default() {
        let schema = json!({"type": "number", "default": 42});
        assert_eq!(example_value_for_param("count", &schema), "42");
    }

    #[test]
    fn test_example_value_string_heuristics() {
        assert_eq!(
            example_value_for_param("file_path", &json!({"type": "string"})),
            "/tmp/example.txt"
        );
        assert_eq!(
            example_value_for_param("query", &json!({"type": "string"})),
            "search term"
        );
        assert_eq!(
            example_value_for_param("url", &json!({"type": "string"})),
            "https://example.com"
        );
    }

    #[test]
    fn test_example_value_number_types() {
        assert_eq!(
            example_value_for_param("temperature", &json!({"type": "number"})),
            "0.7"
        );
        assert_eq!(
            example_value_for_param("limit", &json!({"type": "integer"})),
            "5"
        );
    }

    #[test]
    fn test_generate_fence_example() {
        let tool = ToolInfo {
            name: "search".to_string(),
            category: "test".to_string(),
            description: "Search for items".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"},
                },
                "required": ["query"],
            }),
        };
        let example = generate_fence_example(&tool);
        assert!(example.starts_with("```search\n"));
        assert!(example.contains("query: search term"));
        assert!(example.ends_with("```"));
    }

    #[test]
    fn test_example_value_object_with_properties() {
        let schema = json!({
            "type": "object",
            "properties": {
                "Type": {"type": "string", "description": "Action type"},
                "ColonistId": {"type": "string"}
            },
            "required": ["Type"]
        });
        let val = example_value_for_param("Action", &schema);
        assert!(val.contains("\"Type\""), "Should contain Type key: {val}");
        assert!(
            !val.contains("\"key\""),
            "Should not contain generic key: {val}"
        );
    }

    #[test]
    fn test_example_value_object_without_properties() {
        let schema = json!({"type": "object"});
        let val = example_value_for_param("data", &schema);
        assert_eq!(val, "{\"key\": \"value\"}");
    }

    #[test]
    fn test_generate_fence_example_nested_object() {
        let tool = ToolInfo {
            name: "step".to_string(),
            category: "game".to_string(),
            description: "Execute action".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "AgentId": {"type": "string"},
                    "Action": {
                        "type": "object",
                        "properties": {
                            "Type": {"type": "string"}
                        },
                        "required": ["Type"]
                    }
                },
                "required": ["AgentId", "Action"],
            }),
        };
        let example = generate_fence_example(&tool);
        assert!(
            example.contains("Action: {\"Type\""),
            "Should show nested Type: {example}"
        );
    }

    #[test]
    fn test_generate_fence_example_no_params() {
        let tool = ToolInfo {
            name: "get_time".to_string(),
            category: "test".to_string(),
            description: "Get current time".to_string(),
            schema: json!({"type": "object", "properties": {}}),
        };
        let example = generate_fence_example(&tool);
        assert_eq!(example, "```get_time\n```");
    }
}
