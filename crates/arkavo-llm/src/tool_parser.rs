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
}
