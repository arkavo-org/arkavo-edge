//! Narrating a batch of tool outputs back as user text.
//!
//! Used only when the turn's calls never reached the provider — a Responses
//! turn whose items hold no `function_call`, or calls a loop recovered from
//! prose — so there is no call id to pair a tool-role message against. A failed
//! call also carries its schema here, which is the model's only chance to see
//! the parameter shape it got wrong.

use crate::tool_executor::ToolExecutionResult;
use crate::tool_result::bounded_tool_output;
use std::fmt::Write;

/// Format a turn's tool results as the user-role summary the next request reads.
pub fn tool_result_summary(results: &[ToolExecutionResult]) -> String {
    let mut formatted = String::from("Tool execution results:\n\n");

    for result in results {
        let _ = writeln!(formatted, "Tool: {}", result.tool_name);
        if result.success {
            let json =
                serde_json::to_string_pretty(&result.result).unwrap_or_else(|_| "{}".to_string());
            let _ = writeln!(formatted, "Result: {}", bounded_tool_output(json));
        } else {
            let error = result.error.as_deref().unwrap_or("Unknown error");
            let _ = writeln!(
                formatted,
                "Error: {}",
                bounded_tool_output(error.to_string())
            );
            if let Some(schema) = &result.schema_hint {
                write_schema_hint(&mut formatted, schema);
            }
        }
        formatted.push('\n');
    }

    formatted
}

/// Spell the tool's parameters out in prose; a raw JSON Schema is harder for a
/// small model to act on than a labelled list.
fn write_schema_hint(formatted: &mut String, schema: &serde_json::Value) {
    let _ = writeln!(
        formatted,
        "\nTo fix this error, use the correct parameter format:"
    );
    if let Some(description) = schema.get("description").and_then(|v| v.as_str()) {
        let _ = writeln!(formatted, "Description: {description}");
    }
    let Some(properties) = schema
        .get("parameters")
        .and_then(|params| params.get("properties"))
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    let required: Vec<&str> = schema
        .get("parameters")
        .and_then(|params| params.get("required"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let _ = writeln!(formatted, "Required parameters:");
    for name in &required {
        if let Some(property) = properties.get(*name) {
            write_parameter(formatted, name, property);
        }
    }

    let optional: Vec<_> = properties
        .iter()
        .filter(|(name, _)| !required.contains(&name.as_str()))
        .collect();
    if !optional.is_empty() {
        let _ = writeln!(formatted, "Optional parameters:");
        for (name, property) in optional {
            write_parameter(formatted, name, property);
        }
    }
}

fn write_parameter(formatted: &mut String, name: &str, property: &serde_json::Value) {
    let kind = property
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("any");
    let description = property
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let _ = writeln!(formatted, "  - {name} ({kind}): {description}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_result::MAX_TOOL_RESULT_BYTES;

    fn failed(error: &str, schema_hint: Option<serde_json::Value>) -> ToolExecutionResult {
        ToolExecutionResult {
            tool_name: "read_file".to_string(),
            call_id: None,
            result: serde_json::json!(null),
            success: false,
            error: Some(error.to_string()),
            schema_hint,
        }
    }

    #[test]
    fn summary_truncates_unicode_results_without_panicking() {
        let result = ToolExecutionResult {
            tool_name: "read_file".to_string(),
            call_id: None,
            result: serde_json::json!("界".repeat(MAX_TOOL_RESULT_BYTES)),
            success: true,
            error: None,
            schema_hint: None,
        };
        let summary = tool_result_summary(&[result]);
        assert!(summary.contains("OUTPUT TRUNCATED"));
        assert!(summary.len() < MAX_TOOL_RESULT_BYTES + 1_000);
    }

    #[test]
    fn summary_truncates_unicode_errors_without_panicking() {
        let summary = tool_result_summary(&[failed(&"é".repeat(MAX_TOOL_RESULT_BYTES), None)]);
        assert!(summary.contains("OUTPUT TRUNCATED"));
        assert!(summary.len() < MAX_TOOL_RESULT_BYTES + 1_000);
    }

    #[test]
    fn failed_calls_carry_the_parameter_shape_they_got_wrong() {
        let summary = tool_result_summary(&[failed(
            "missing path",
            Some(serde_json::json!({
                "description": "Read a file",
                "parameters": {
                    "properties": {
                        "path": {"type": "string", "description": "file to read"},
                        "encoding": {"type": "string", "description": "text encoding"}
                    },
                    "required": ["path"]
                }
            })),
        )]);
        assert!(summary.contains("Error: missing path"));
        assert!(summary.contains("Description: Read a file"));
        assert!(summary.contains("Required parameters:\n  - path (string): file to read"));
        assert!(summary.contains("Optional parameters:\n  - encoding (string): text encoding"));
    }

    #[test]
    fn successful_results_are_rendered_without_a_schema_section() {
        let result = ToolExecutionResult {
            tool_name: "clock".to_string(),
            call_id: None,
            result: serde_json::json!({"now": "noon"}),
            success: true,
            error: None,
            schema_hint: Some(serde_json::json!({"description": "unused"})),
        };
        let summary = tool_result_summary(&[result]);
        assert!(summary.starts_with("Tool execution results:\n\nTool: clock\nResult: {"));
        assert!(!summary.contains("unused"));
    }
}
