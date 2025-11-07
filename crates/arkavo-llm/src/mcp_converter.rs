use arkavo_mcp_tools::registry::ToolInfo;
use serde_json::{Value, json};
use std::fmt::Write as _;

/// Tool definition in provider-agnostic format
/// Can be serialized to any provider's specific format
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub struct McpConverter;

impl McpConverter {
    /// Convert MCP ToolInfo to provider-agnostic format
    /// This can then be serialized to match any provider's API
    pub fn to_provider_tools(tools: &[ToolInfo]) -> Vec<ProviderTool> {
        tools
            .iter()
            .map(|tool| ProviderTool {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.schema.clone(),
            })
            .collect()
    }

    /// Convert to Gemini JSON format (runtime, no feature gate)
    pub fn to_gemini_format(tools: &[ToolInfo]) -> Value {
        if tools.is_empty() {
            return json!([]);
        }

        let function_declarations: Vec<Value> = tools
            .iter()
            .map(|tool| {
                // Convert schema to Gemini-compatible format (replace const with enum)
                let gemini_schema = Self::make_gemini_compatible(&tool.schema);

                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": gemini_schema,
                })
            })
            .collect();

        json!([{
            "functionDeclarations": function_declarations
        }])
    }

    /// Make JSON Schema compatible with Gemini API
    /// Gemini doesn't support "const", so convert to "enum" with single value
    fn make_gemini_compatible(schema: &Value) -> Value {
        match schema {
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (key, value) in map {
                    if key == "const" {
                        // Replace const with enum containing single value
                        new_map.insert("enum".to_string(), json!([value]));
                    } else {
                        // Recursively process nested values
                        new_map.insert(key.clone(), Self::make_gemini_compatible(value));
                    }
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => {
                Value::Array(arr.iter().map(Self::make_gemini_compatible).collect())
            }
            _ => schema.clone(),
        }
    }

    /// Convert to DeepSeek/Anthropic JSON format (runtime, no feature gate)
    pub fn to_anthropic_format(tools: &[ToolInfo]) -> Value {
        let tool_defs: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.schema,
                })
            })
            .collect();

        json!(tool_defs)
    }

    /// Convert to OpenAI JSON format (always available)
    pub fn to_openai_format(tools: &[ToolInfo]) -> Value {
        let functions: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.schema,
                })
            })
            .collect();

        json!(functions)
    }

    /// Convert MCP ToolInfo to XML prompt format for local models
    pub fn to_xml_prompt(tools: &[ToolInfo]) -> String {
        if tools.is_empty() {
            return String::new();
        }

        let mut xml = String::from("\n\nYou have access to these tools:\n<tools>\n");

        for tool in tools {
            let _ = writeln!(xml, "  <tool name=\"{}\">", tool.name);
            let _ = writeln!(
                xml,
                "    <description>{}</description>",
                Self::escape_xml(&tool.description)
            );
            let _ = writeln!(
                xml,
                "    <category>{}</category>",
                Self::escape_xml(&tool.category)
            );

            let params_str =
                serde_json::to_string_pretty(&tool.schema).unwrap_or_else(|_| "{}".to_string());
            let _ = writeln!(
                xml,
                "    <parameters>{}</parameters>",
                Self::escape_xml(&params_str)
            );
            xml.push_str("  </tool>\n");
        }

        xml.push_str("</tools>\n\n");
        xml.push_str("To call a tool, respond with:\n");
        xml.push_str("<tool_call>\n");
        xml.push_str("  <name>tool_name</name>\n");
        xml.push_str("  <arguments>{\"key\": \"value\"}</arguments>\n");
        xml.push_str("</tool_call>\n\n");
        xml.push_str("After receiving tool results, continue the conversation naturally.\n");

        xml
    }

    /// Convert MCP ToolInfo to JSON prompt format (alternative to XML)
    pub fn to_json_prompt(tools: &[ToolInfo]) -> String {
        if tools.is_empty() {
            return String::new();
        }

        let tools_json: Vec<Value> = tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "category": tool.category,
                    "parameters": tool.schema,
                })
            })
            .collect();

        let mut prompt = String::from("\n\nAvailable tools:\n");
        prompt.push_str(
            &serde_json::to_string_pretty(&tools_json).unwrap_or_else(|_| "[]".to_string()),
        );
        prompt.push_str("\n\nTo call a tool, respond with JSON in this format:\n");
        prompt.push_str(
            "{\"tool_call\": {\"name\": \"tool_name\", \"arguments\": {\"key\": \"value\"}}}\n\n",
        );
        prompt.push_str("After receiving tool results, continue the conversation naturally.\n");

        prompt
    }

    /// Escape XML special characters
    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_tool() -> ToolInfo {
        ToolInfo {
            name: "test_tool".to_string(),
            category: "Testing".to_string(),
            description: "A test tool for unit tests".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    #[test]
    fn test_to_provider_tools() {
        let tools = vec![create_test_tool()];
        let provider_tools = McpConverter::to_provider_tools(&tools);

        assert_eq!(provider_tools.len(), 1);
        assert_eq!(provider_tools[0].name, "test_tool");
        assert_eq!(provider_tools[0].description, "A test tool for unit tests");
    }

    #[test]
    fn test_to_gemini_format() {
        let tools = vec![create_test_tool()];
        let gemini_json = McpConverter::to_gemini_format(&tools);

        assert!(gemini_json.is_array());
        let arr = gemini_json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert!(arr[0]["functionDeclarations"].is_array());
    }

    #[test]
    fn test_to_anthropic_format() {
        let tools = vec![create_test_tool()];
        let anthropic_json = McpConverter::to_anthropic_format(&tools);

        assert!(anthropic_json.is_array());
        let arr = anthropic_json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "test_tool");
        assert!(arr[0]["input_schema"].is_object());
    }

    #[test]
    fn test_to_openai_format() {
        let tools = vec![create_test_tool()];
        let openai_json = McpConverter::to_openai_format(&tools);

        assert!(openai_json.is_array());
        let arr = openai_json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "test_tool");
        assert!(arr[0]["parameters"].is_object());
    }

    #[test]
    fn test_to_xml_prompt() {
        let tools = vec![create_test_tool()];
        let xml = McpConverter::to_xml_prompt(&tools);

        assert!(xml.contains("<tools>"));
        assert!(xml.contains("<tool name=\"test_tool\">"));
        assert!(xml.contains("<description>A test tool for unit tests</description>"));
        assert!(xml.contains("<tool_call>"));
    }

    #[test]
    fn test_to_json_prompt() {
        let tools = vec![create_test_tool()];
        let json_prompt = McpConverter::to_json_prompt(&tools);

        assert!(json_prompt.contains("test_tool"));
        assert!(json_prompt.contains("A test tool for unit tests"));
        assert!(json_prompt.contains("tool_call"));
    }

    #[test]
    fn test_empty_tools() {
        let tools: Vec<ToolInfo> = vec![];

        assert!(McpConverter::to_provider_tools(&tools).is_empty());
        assert!(
            McpConverter::to_gemini_format(&tools)
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            McpConverter::to_anthropic_format(&tools)
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            McpConverter::to_openai_format(&tools)
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(McpConverter::to_xml_prompt(&tools).is_empty());
        assert!(McpConverter::to_json_prompt(&tools).is_empty());
    }

    #[test]
    fn test_xml_escaping() {
        let tool = ToolInfo {
            name: "test".to_string(),
            category: "Test".to_string(),
            description: "Tool with <special> & \"chars\"".to_string(),
            schema: json!({}),
        };

        let xml = McpConverter::to_xml_prompt(&[tool]);
        assert!(xml.contains("&lt;special&gt; &amp; &quot;chars&quot;"));
    }
}
