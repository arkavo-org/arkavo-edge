use regex::Regex;
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolParseError {
    #[error("Invalid JSON in tool call: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("Missing required field: {0}")]
    MissingField(String),
    #[error("Invalid tool call format: {0}")]
    InvalidFormat(String),
    #[error("No tool calls found in response")]
    NoToolCalls,
}

/// Unified tool call representation from any provider
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedToolCall {
    pub tool_name: String,
    pub arguments: Value,
    pub call_id: Option<String>,
}

pub struct ToolParser;

impl ToolParser {
    /// Parse tool calls from Gemini API response
    pub fn parse_gemini(response: &Value) -> Result<Vec<ParsedToolCall>, ToolParseError> {
        let function_calls = response
            .get("functionCall")
            .or_else(|| response.get("functionCalls"))
            .ok_or(ToolParseError::NoToolCalls)?;

        if function_calls.is_array() {
            let calls_array = function_calls
                .as_array()
                .ok_or_else(|| ToolParseError::InvalidFormat("Expected array".to_string()))?;
            calls_array.iter().map(Self::parse_gemini_single).collect()
        } else {
            Ok(vec![Self::parse_gemini_single(function_calls)?])
        }
    }

    fn parse_gemini_single(call: &Value) -> Result<ParsedToolCall, ToolParseError> {
        let name = call
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolParseError::MissingField("name".to_string()))?;

        let args = call
            .get("args")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::default()));

        Ok(ParsedToolCall {
            tool_name: name.to_string(),
            arguments: args,
            call_id: call.get("id").and_then(|v| v.as_str()).map(String::from),
        })
    }

    /// Parse tool calls from Anthropic/DeepSeek API response
    pub fn parse_anthropic(response: &Value) -> Result<Vec<ParsedToolCall>, ToolParseError> {
        let tool_calls = response
            .get("tool_calls")
            .or_else(|| response.get("content"))
            .ok_or(ToolParseError::NoToolCalls)?;

        let calls_array = tool_calls
            .as_array()
            .ok_or_else(|| ToolParseError::InvalidFormat("Expected array".to_string()))?;

        calls_array
            .iter()
            .map(Self::parse_anthropic_single)
            .collect()
    }

    fn parse_anthropic_single(call: &Value) -> Result<ParsedToolCall, ToolParseError> {
        let name = call
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolParseError::MissingField("name".to_string()))?;

        let input = call
            .get("input")
            .ok_or_else(|| ToolParseError::MissingField("input".to_string()))?;

        Ok(ParsedToolCall {
            tool_name: name.to_string(),
            arguments: input.clone(),
            call_id: call.get("id").and_then(|v| v.as_str()).map(String::from),
        })
    }

    /// Parse tool calls from OpenAI API response
    pub fn parse_openai(response: &Value) -> Result<Vec<ParsedToolCall>, ToolParseError> {
        let tool_calls = response
            .get("tool_calls")
            .ok_or(ToolParseError::NoToolCalls)?
            .as_array()
            .ok_or_else(|| ToolParseError::InvalidFormat("Expected array".to_string()))?;

        tool_calls.iter().map(Self::parse_openai_single).collect()
    }

    fn parse_openai_single(call: &Value) -> Result<ParsedToolCall, ToolParseError> {
        let function = call
            .get("function")
            .ok_or_else(|| ToolParseError::MissingField("function".to_string()))?;

        let name = function
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolParseError::MissingField("name".to_string()))?;

        let args_str = function
            .get("arguments")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolParseError::MissingField("arguments".to_string()))?;

        let args: Value = serde_json::from_str(args_str)?;

        Ok(ParsedToolCall {
            tool_name: name.to_string(),
            arguments: args,
            call_id: call.get("id").and_then(|v| v.as_str()).map(String::from),
        })
    }

    /// Parse XML-based tool calls from local model text responses
    pub fn parse_xml(text: &str) -> Result<Vec<ParsedToolCall>, ToolParseError> {
        let mut calls = Vec::new();
        let mut pos = 0;

        while let Some(start) = text[pos..].find("<tool_call>") {
            let start = pos + start;
            let end = text[start..]
                .find("</tool_call>")
                .ok_or_else(|| ToolParseError::InvalidFormat("Unclosed <tool_call>".to_string()))?
                + start;

            let call_xml = &text[start..end + "</tool_call>".len()];
            calls.push(Self::parse_xml_single(call_xml)?);
            pos = end + "</tool_call>".len();
        }

        if calls.is_empty() {
            return Err(ToolParseError::NoToolCalls);
        }

        Ok(calls)
    }

    fn parse_xml_single(xml: &str) -> Result<ParsedToolCall, ToolParseError> {
        let name = Self::extract_xml_tag(xml, "name")?;
        let args_str = Self::extract_xml_tag(xml, "arguments")?;
        let args: Value = serde_json::from_str(&args_str)?;

        Ok(ParsedToolCall {
            tool_name: name,
            arguments: args,
            call_id: None,
        })
    }

    /// Parse JSON-based tool calls from local model text responses
    pub fn parse_json(text: &str) -> Result<Vec<ParsedToolCall>, ToolParseError> {
        let json: Value = serde_json::from_str(text)?;

        if let Some(tool_call) = json.get("tool_call") {
            Ok(vec![Self::parse_json_single(tool_call)?])
        } else if json.is_array() {
            let calls_array = json
                .as_array()
                .ok_or_else(|| ToolParseError::InvalidFormat("Expected array".to_string()))?;
            calls_array.iter().map(Self::parse_json_single).collect()
        } else {
            Err(ToolParseError::InvalidFormat(
                "Expected tool_call object or array".to_string(),
            ))
        }
    }

    fn parse_json_single(call: &Value) -> Result<ParsedToolCall, ToolParseError> {
        let name = call
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolParseError::MissingField("name".to_string()))?;

        let args = call
            .get("arguments")
            .ok_or_else(|| ToolParseError::MissingField("arguments".to_string()))?;

        Ok(ParsedToolCall {
            tool_name: name.to_string(),
            arguments: args.clone(),
            call_id: None,
        })
    }

    fn extract_xml_tag(xml: &str, tag: &str) -> Result<String, ToolParseError> {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");

        let start = xml
            .find(&open)
            .ok_or_else(|| ToolParseError::MissingField(tag.to_string()))?
            + open.len();
        let end = xml[start..]
            .find(&close)
            .ok_or_else(|| ToolParseError::InvalidFormat(format!("Unclosed <{tag}>")))?
            + start;

        Ok(xml[start..end].trim().to_string())
    }

    /// Parse fence-based tool calls from local model text responses
    /// Format: ```tool_name\nkey: value\nkey2: value2\n```
    /// Also handles: ```\ntool_name\nkey: value\n```
    pub fn parse_fence(text: &str) -> Result<Vec<ParsedToolCall>, ToolParseError> {
        let mut calls = Vec::new();

        // Primary pattern: ```tool_name\n...content...\n```
        // Tool names are lowercase with underscores, no spaces
        let primary_pattern = Regex::new(r"```([a-z][a-z0-9_]*)\s*\n([\s\S]*?)```")
            .map_err(|e| ToolParseError::InvalidFormat(format!("Regex error: {e}")))?;

        for cap in primary_pattern.captures_iter(text) {
            if let (Some(name), Some(content)) = (cap.get(1), cap.get(2)) {
                let arguments = Self::parse_key_value_content(content.as_str())?;
                calls.push(ParsedToolCall {
                    tool_name: name.as_str().to_string(),
                    arguments,
                    call_id: None,
                });
            }
        }

        // Fallback pattern: ```\ntool_name\nkey: value\n```
        // Some models put empty fence then tool name on next line
        if calls.is_empty() {
            let fallback_pattern = Regex::new(r"```\s*\n([a-z][a-z0-9_]*)\s*\n([\s\S]*?)```")
                .map_err(|e| ToolParseError::InvalidFormat(format!("Regex error: {e}")))?;

            for cap in fallback_pattern.captures_iter(text) {
                if let (Some(name), Some(content)) = (cap.get(1), cap.get(2)) {
                    let arguments = Self::parse_key_value_content(content.as_str())?;
                    calls.push(ParsedToolCall {
                        tool_name: name.as_str().to_string(),
                        arguments,
                        call_id: None,
                    });
                }
            }
        }

        if calls.is_empty() {
            return Err(ToolParseError::NoToolCalls);
        }

        Ok(calls)
    }

    /// Parse key: value pairs from fence content into JSON Value
    fn parse_key_value_content(content: &str) -> Result<Value, ToolParseError> {
        let mut map = Map::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Find first colon to split key: value
            if let Some(colon_pos) = line.find(':') {
                let key = line[..colon_pos].trim().to_string();
                let value = line[colon_pos + 1..].trim();

                // Skip empty keys
                if key.is_empty() {
                    continue;
                }

                // Try to parse as JSON value, fall back to string
                let json_value = Self::infer_value_type(value);
                map.insert(key, json_value);
            }
        }

        Ok(Value::Object(map))
    }

    /// Infer JSON type from string value
    fn infer_value_type(s: &str) -> Value {
        // Empty string
        if s.is_empty() {
            return Value::String(String::new());
        }

        // Boolean
        if s.eq_ignore_ascii_case("true") {
            return Value::Bool(true);
        }
        if s.eq_ignore_ascii_case("false") {
            return Value::Bool(false);
        }

        // Null
        if s.eq_ignore_ascii_case("null") || s.eq_ignore_ascii_case("none") {
            return Value::Null;
        }

        // Integer
        if let Ok(n) = s.parse::<i64>() {
            return Value::Number(n.into());
        }

        // Float
        if let Ok(n) = s.parse::<f64>()
            && let Some(num) = serde_json::Number::from_f64(n)
        {
            return Value::Number(num);
        }

        // JSON array or object (for complex values)
        if ((s.starts_with('[') && s.ends_with(']')) || (s.starts_with('{') && s.ends_with('}')))
            && let Ok(v) = serde_json::from_str(s)
        {
            return v;
        }

        // Default: string
        Value::String(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_gemini() {
        let response = json!({
            "functionCall": {
                "name": "get_weather",
                "args": {"city": "London"},
                "id": "call_123"
            }
        });

        let calls = ToolParser::parse_gemini(&response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "London");
        assert_eq!(calls[0].call_id, Some("call_123".to_string()));
    }

    #[test]
    fn test_parse_anthropic() {
        let response = json!({
            "tool_calls": [{
                "name": "search",
                "input": {"query": "rust"},
                "id": "call_456"
            }]
        });

        let calls = ToolParser::parse_anthropic(&response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "search");
        assert_eq!(calls[0].arguments["query"], "rust");
    }

    #[test]
    fn test_parse_openai() {
        let response = json!({
            "tool_calls": [{
                "id": "call_789",
                "function": {
                    "name": "calculate",
                    "arguments": r#"{"x": 5, "y": 3}"#
                }
            }]
        });

        let calls = ToolParser::parse_openai(&response).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "calculate");
        assert_eq!(calls[0].arguments["x"], 5);
    }

    #[test]
    fn test_parse_xml() {
        let text = r#"
Some text before
<tool_call>
  <name>test_tool</name>
  <arguments>{"key": "value"}</arguments>
</tool_call>
Some text after
"#;

        let calls = ToolParser::parse_xml(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "test_tool");
        assert_eq!(calls[0].arguments["key"], "value");
        assert_eq!(calls[0].call_id, None);
    }

    #[test]
    fn test_parse_json() {
        let text = r#"{"tool_call": {"name": "search", "arguments": {"q": "test"}}}"#;

        let calls = ToolParser::parse_json(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "search");
        assert_eq!(calls[0].arguments["q"], "test");
    }

    #[test]
    fn test_parse_multiple_xml() {
        let text = r#"
<tool_call>
  <name>tool1</name>
  <arguments>{"a": 1}</arguments>
</tool_call>
<tool_call>
  <name>tool2</name>
  <arguments>{"b": 2}</arguments>
</tool_call>
"#;

        let calls = ToolParser::parse_xml(text).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "tool1");
        assert_eq!(calls[1].tool_name, "tool2");
    }

    #[test]
    fn test_parse_fence_simple() {
        let text = r#"
Let me check the weather for you.

```get_weather
location: Columbia, MD
unit: fahrenheit
```

I'll get that information now.
"#;

        let calls = ToolParser::parse_fence(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "get_weather");
        assert_eq!(calls[0].arguments["location"], "Columbia, MD");
        assert_eq!(calls[0].arguments["unit"], "fahrenheit");
    }

    #[test]
    fn test_parse_fence_multiple_tools() {
        let text = r#"
```read_file
path: /src/main.rs
```

```write_file
path: /src/output.txt
content: Hello World
```
"#;

        let calls = ToolParser::parse_fence(text).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "read_file");
        assert_eq!(calls[0].arguments["path"], "/src/main.rs");
        assert_eq!(calls[1].tool_name, "write_file");
        assert_eq!(calls[1].arguments["path"], "/src/output.txt");
    }

    #[test]
    fn test_parse_fence_type_inference() {
        let text = r#"
```calculate
x: 42
y: 3.14
enabled: true
disabled: false
nothing: null
```
"#;

        let calls = ToolParser::parse_fence(text).unwrap();
        assert_eq!(calls[0].arguments["x"], 42);
        assert_eq!(calls[0].arguments["y"], 3.14);
        assert_eq!(calls[0].arguments["enabled"], true);
        assert_eq!(calls[0].arguments["disabled"], false);
        assert!(calls[0].arguments["nothing"].is_null());
    }

    #[test]
    fn test_parse_fence_no_tools() {
        let text = "Just a regular response without any tool calls.";
        let result = ToolParser::parse_fence(text);
        assert!(matches!(result, Err(ToolParseError::NoToolCalls)));
    }

    #[test]
    fn test_parse_fence_with_json_array() {
        let text = r#"
```search_files
patterns: ["*.rs", "*.toml"]
path: /src
```
"#;

        let calls = ToolParser::parse_fence(text).unwrap();
        assert_eq!(calls[0].tool_name, "search_files");
        let patterns = calls[0].arguments["patterns"].as_array().unwrap();
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0], "*.rs");
    }

    #[test]
    fn test_parse_fence_windows_path() {
        // Windows paths have colons which could confuse the parser
        let text = r#"
```read_file
path: C:\Users\test\file.txt
```
"#;

        let calls = ToolParser::parse_fence(text).unwrap();
        // First colon splits key, rest is value
        assert_eq!(calls[0].arguments["path"], "C:\\Users\\test\\file.txt");
    }

    #[test]
    fn test_infer_value_type() {
        assert_eq!(ToolParser::infer_value_type("true"), Value::Bool(true));
        assert_eq!(ToolParser::infer_value_type("TRUE"), Value::Bool(true));
        assert_eq!(ToolParser::infer_value_type("false"), Value::Bool(false));
        assert_eq!(ToolParser::infer_value_type("42"), json!(42));
        assert_eq!(ToolParser::infer_value_type("3.14"), json!(3.14));
        assert_eq!(ToolParser::infer_value_type("hello world"), json!("hello world"));
        assert_eq!(ToolParser::infer_value_type("null"), Value::Null);
        assert_eq!(ToolParser::infer_value_type("none"), Value::Null);
    }

    #[test]
    fn test_parse_fence_fallback_pattern() {
        // Model output where tool name is on separate line
        let text = r#"
```
get_weather
location: New York
```
"#;

        let calls = ToolParser::parse_fence(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "get_weather");
        assert_eq!(calls[0].arguments["location"], "New York");
    }

    #[test]
    fn test_parse_fence_with_empty_first_fence() {
        // Model output with empty fence before actual content
        let text = r#"
```
```
get_weather
location: New York
```
"#;

        let calls = ToolParser::parse_fence(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "get_weather");
    }
}
