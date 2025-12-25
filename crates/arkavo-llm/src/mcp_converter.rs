use arkavo_mcp_tools::registry::{MinimalToolInfo, ToolInfo};
use serde_json::{Value, json};
use std::fmt::Write as _;

/// Tool call format for local models
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LocalToolFormat {
    /// XML format: <tool_call><name>...</name><arguments>...</arguments></tool_call>
    Xml,
    /// JSON format: {"tool_call": {"name": "...", "arguments": {...}}}
    Json,
    /// Fence format: ```tool_name\nkey: value\n``` (most reliable for small models)
    #[default]
    Fence,
}

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
    /// - Gemini doesn't support "const", so convert to "enum" with single value
    /// - Strip out non-JSON-Schema fields that may leak from MCP ToolSchema
    fn make_gemini_compatible(schema: &Value) -> Value {
        // Invalid fields that may leak from MCP ToolSchema into parameters
        const INVALID_FIELDS: &[&str] = &["name", "aliases", "parameters", "category"];

        match schema {
            Value::Object(map) => {
                let mut new_map = serde_json::Map::new();
                for (key, value) in map {
                    // Skip fields that are not valid JSON Schema
                    if INVALID_FIELDS.contains(&key.as_str()) {
                        continue;
                    }
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

    /// Make JSON Schema compatible with Anthropic Claude API
    /// Anthropic doesn't support oneOf, allOf, anyOf at the top level of input_schema
    /// This function flattens these constructs to simple types
    fn make_anthropic_compatible(schema: &Value) -> Value {
        match schema {
            Value::Object(map) => {
                // Check for oneOf/anyOf/allOf at this level
                if let Some(one_of) = map.get("oneOf").and_then(|v| v.as_array()) {
                    // Pick the first option that has a type (prefer string/number over complex)
                    return Self::pick_best_from_union(one_of);
                }
                if let Some(any_of) = map.get("anyOf").and_then(|v| v.as_array()) {
                    return Self::pick_best_from_union(any_of);
                }
                if let Some(all_of) = map.get("allOf").and_then(|v| v.as_array()) {
                    // For allOf, merge all schemas together
                    return Self::merge_all_of(all_of);
                }

                // No union types, recursively process properties
                let mut new_map = serde_json::Map::new();
                for (key, value) in map {
                    // Skip keys that are union type keywords - we already handled them
                    if key == "oneOf" || key == "anyOf" || key == "allOf" {
                        continue;
                    }
                    // Recursively sanitize nested objects
                    new_map.insert(key.clone(), Self::make_anthropic_compatible(value));
                }
                Value::Object(new_map)
            }
            Value::Array(arr) => {
                Value::Array(arr.iter().map(Self::make_anthropic_compatible).collect())
            }
            _ => schema.clone(),
        }
    }

    /// Pick the best schema from a oneOf/anyOf union
    /// Prefers simple types (string, number, integer) over complex objects
    fn pick_best_from_union(options: &[Value]) -> Value {
        // Priority: string > number > integer > boolean > object > first
        let priority_types = ["string", "number", "integer", "boolean"];

        for ptype in priority_types {
            for opt in options {
                if let Some(t) = opt.get("type").and_then(|v| v.as_str())
                    && t == ptype
                {
                    return Self::make_anthropic_compatible(opt);
                }
            }
        }

        // No simple type found, use first option
        if let Some(first) = options.first() {
            Self::make_anthropic_compatible(first)
        } else {
            json!({"type": "string"})
        }
    }

    /// Merge allOf schemas into a single schema
    fn merge_all_of(schemas: &[Value]) -> Value {
        let mut merged = serde_json::Map::new();
        let mut merged_props = serde_json::Map::new();
        let mut merged_required: Vec<Value> = Vec::new();

        for schema in schemas {
            if let Some(obj) = schema.as_object() {
                for (key, value) in obj {
                    match key.as_str() {
                        "properties" => {
                            if let Some(props) = value.as_object() {
                                for (pk, pv) in props {
                                    merged_props.insert(pk.clone(), pv.clone());
                                }
                            }
                        }
                        "required" => {
                            if let Some(req) = value.as_array() {
                                merged_required.extend(req.iter().cloned());
                            }
                        }
                        _ => {
                            merged.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
        }

        if !merged_props.is_empty() {
            merged.insert("properties".to_string(), Value::Object(merged_props));
        }
        if !merged_required.is_empty() {
            merged.insert("required".to_string(), Value::Array(merged_required));
        }

        Self::make_anthropic_compatible(&Value::Object(merged))
    }

    /// Convert to DeepSeek/Anthropic JSON format (runtime, no feature gate)
    /// Sanitizes schemas to remove oneOf/anyOf/allOf which Anthropic doesn't support
    pub fn to_anthropic_format(tools: &[ToolInfo]) -> Value {
        let tool_defs: Vec<Value> = tools
            .iter()
            .map(|tool| {
                // Sanitize schema for Anthropic compatibility
                let sanitized_schema = Self::make_anthropic_compatible(&tool.schema);
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": sanitized_schema,
                })
            })
            .collect();

        json!(tool_defs)
    }

    /// Convert MinimalToolInfo to Anthropic format (for progressive disclosure)
    /// Sanitizes schemas to remove oneOf/anyOf/allOf which Anthropic doesn't support
    pub fn to_anthropic_format_minimal(tools: &[MinimalToolInfo]) -> Value {
        let tool_defs: Vec<Value> = tools
            .iter()
            .map(|tool| {
                let schema = tool
                    .schema
                    .clone()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                // Sanitize schema for Anthropic compatibility
                let sanitized_schema = Self::make_anthropic_compatible(&schema);
                json!({
                    "name": tool.name,
                    "description": tool.description.clone().unwrap_or_default(),
                    "input_schema": sanitized_schema,
                })
            })
            .collect();

        json!(tool_defs)
    }

    /// Convert MinimalToolInfo to Gemini format (for progressive disclosure)
    pub fn to_gemini_format_minimal(tools: &[MinimalToolInfo]) -> Value {
        if tools.is_empty() {
            return json!([]);
        }

        let function_declarations: Vec<Value> = tools
            .iter()
            .map(|tool| {
                let schema = tool
                    .schema
                    .clone()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                let gemini_schema = Self::make_gemini_compatible(&schema);

                json!({
                    "name": tool.name,
                    "description": tool.description.clone().unwrap_or_default(),
                    "parameters": gemini_schema,
                })
            })
            .collect();

        json!([{
            "functionDeclarations": function_declarations
        }])
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

    /// Convert MCP ToolInfo to fence-based prompt format for small models
    /// Uses markdown code fence with tool name as language tag and simple key: value pairs
    pub fn to_fence_prompt(tools: &[ToolInfo]) -> String {
        if tools.is_empty() {
            return String::new();
        }

        let mut prompt = String::new();

        // Be very direct and imperative for small models
        prompt.push_str("\n\nYou have tools. Call them using code fences.\n\n");
        prompt.push_str("Tools:\n");

        for tool in tools {
            let _ = writeln!(prompt, "- {}: {}", tool.name, tool.description);
        }

        prompt.push_str("\nFormat:\n```<tool>\n<param>: <value>\n```\n\n");

        // Add concrete examples for each tool
        prompt.push_str("Examples:\n");
        for tool in tools.iter().take(2) {
            let _ = writeln!(prompt, "```{}", tool.name);
            if let Some(props) = tool.schema.get("properties").and_then(|p| p.as_object()) {
                let required: Vec<&str> = tool
                    .schema
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();

                // Show required params with realistic example values
                for (name, _prop) in props.iter() {
                    if required.contains(&name.as_str()) {
                        let example = match name.as_str() {
                            "location" => "Tokyo",
                            "path" | "file_path" => "/tmp/file.txt",
                            "query" => "search term",
                            "url" => "https://example.com",
                            "command" => "ls -la",
                            "message" | "text" | "content" => "Hello world",
                            "x" | "y" | "z" => "100",
                            _ => "value",
                        };
                        let _ = writeln!(prompt, "{name}: {example}");
                    }
                }
            }
            prompt.push_str("```\n");
        }

        prompt.push_str("\nRespond with ONLY the code fence when using a tool.\n");

        prompt
    }

    /// Convert MCP ToolInfo to prompt format based on specified format
    pub fn to_local_prompt(tools: &[ToolInfo], format: LocalToolFormat) -> String {
        match format {
            LocalToolFormat::Xml => Self::to_xml_prompt(tools),
            LocalToolFormat::Json => Self::to_json_prompt(tools),
            LocalToolFormat::Fence => Self::to_fence_prompt(tools),
        }
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

    #[test]
    fn test_to_fence_prompt() {
        let tools = vec![create_test_tool()];
        let prompt = McpConverter::to_fence_prompt(&tools);

        assert!(prompt.contains("- test_tool:"));
        assert!(prompt.contains("A test tool for unit tests"));
        assert!(prompt.contains("```<tool>"));
        assert!(prompt.contains("Examples:"));
        assert!(prompt.contains("```test_tool"));
    }

    #[test]
    fn test_to_fence_prompt_empty() {
        let tools: Vec<ToolInfo> = vec![];
        assert!(McpConverter::to_fence_prompt(&tools).is_empty());
    }

    #[test]
    fn test_to_fence_prompt_shows_example() {
        let tool = ToolInfo {
            name: "test".to_string(),
            category: "Test".to_string(),
            description: "Test tool".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string", "description": "A location param"},
                    "optional_param": {"type": "string", "description": "An optional param"}
                },
                "required": ["location"]
            }),
        };

        let prompt = McpConverter::to_fence_prompt(&[tool]);
        // Should contain the tool name as example
        assert!(prompt.contains("```test"));
        // Should contain realistic example value for location
        assert!(prompt.contains("location: Tokyo"));
    }

    #[test]
    fn test_to_local_prompt_format_selection() {
        let tools = vec![create_test_tool()];

        let xml_prompt = McpConverter::to_local_prompt(&tools, LocalToolFormat::Xml);
        assert!(xml_prompt.contains("<tools>"));

        let fence_prompt = McpConverter::to_local_prompt(&tools, LocalToolFormat::Fence);
        assert!(fence_prompt.contains("```<tool>"));
        assert!(fence_prompt.contains("Examples:"));

        let json_prompt = McpConverter::to_local_prompt(&tools, LocalToolFormat::Json);
        assert!(json_prompt.contains("tool_call"));
    }

    #[test]
    fn test_anthropic_schema_sanitization_oneof() {
        // Test that oneOf is converted to a simple type (picks string over number)
        let tool = ToolInfo {
            name: "test".to_string(),
            category: "Test".to_string(),
            description: "Tool with oneOf".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "oneOf": [
                            {"type": "string"},
                            {"type": "number"}
                        ]
                    }
                }
            }),
        };

        let result = McpConverter::to_anthropic_format(&[tool]);
        let schema = &result[0]["input_schema"];

        // Should not contain oneOf
        assert!(!schema.to_string().contains("oneOf"));
        // The id property should be converted to string type
        assert_eq!(schema["properties"]["id"]["type"], "string");
    }

    #[test]
    fn test_anthropic_schema_sanitization_anyof() {
        // Test that anyOf is also converted
        let tool = ToolInfo {
            name: "test".to_string(),
            category: "Test".to_string(),
            description: "Tool with anyOf".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "value": {
                        "anyOf": [
                            {"type": "integer"},
                            {"type": "string"}
                        ]
                    }
                }
            }),
        };

        let result = McpConverter::to_anthropic_format(&[tool]);
        let schema = &result[0]["input_schema"];

        // Should not contain anyOf
        assert!(!schema.to_string().contains("anyOf"));
        // The value property should be converted to string type (preferred over integer)
        assert_eq!(schema["properties"]["value"]["type"], "string");
    }

    #[test]
    fn test_anthropic_schema_sanitization_nested() {
        // Test that nested oneOf is also handled
        let tool = ToolInfo {
            name: "test".to_string(),
            category: "Test".to_string(),
            description: "Tool with nested oneOf".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "data": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "oneOf": [
                                    {"type": "string"},
                                    {"type": "number"}
                                ]
                            }
                        }
                    }
                }
            }),
        };

        let result = McpConverter::to_anthropic_format(&[tool]);
        let schema_str = result[0]["input_schema"].to_string();

        // Should not contain oneOf anywhere in the schema
        assert!(!schema_str.contains("oneOf"));
    }

    #[test]
    fn test_anthropic_schema_sanitization_top_level_oneof() {
        // Test that top-level oneOf (the actual error case) is handled
        let tool = ToolInfo {
            name: "test".to_string(),
            category: "Test".to_string(),
            description: "Tool with top-level oneOf".to_string(),
            schema: json!({
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {"a": {"type": "string"}}
                    },
                    {
                        "type": "object",
                        "properties": {"b": {"type": "number"}}
                    }
                ]
            }),
        };

        let result = McpConverter::to_anthropic_format(&[tool]);
        let schema = &result[0]["input_schema"];

        // Should not contain oneOf
        assert!(!schema.to_string().contains("oneOf"));
        // Should have picked the first object option
        assert_eq!(schema["type"], "object");
    }
}
