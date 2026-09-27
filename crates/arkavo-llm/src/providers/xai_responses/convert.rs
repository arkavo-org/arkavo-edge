use crate::tool_parser::ParsedToolCall;
use crate::{Message, Role};
use serde_json::{Value, json};

/// Known Responses built-in tool type strings (no nested `function` object).
const BUILTIN_TOOL_TYPES: &[&str] = &[
    "web_search",
    "web_search_preview",
    "code_interpreter",
    "file_search",
    "computer_use",
    "mcp",
];

/// Convert internal messages into Responses `input` items.
pub(super) fn convert_input(messages: &[Message]) -> Value {
    let items: Vec<Value> = messages
        .iter()
        .map(|msg| match &msg.role {
            Role::Tool => {
                let call_id = msg
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "call_unknown".to_string());
                json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": msg.content,
                })
            }
            Role::Assistant if !msg.tool_calls.is_empty() => {
                // Replay prior assistant tool calls as function_call items
                // so the model sees the full trajectory when store=false.
                let mut parts = Vec::new();
                if !msg.content.is_empty() {
                    parts.push(json!({
                        "role": "assistant",
                        "content": msg.content,
                    }));
                }
                for tc in &msg.tool_calls {
                    parts.push(json!({
                        "type": "function_call",
                        "call_id": tc.id.clone().unwrap_or_default(),
                        "name": tc.name,
                        "arguments": tc.arguments,
                    }));
                }
                json!({ "_parts": parts })
            }
            role => {
                let role_str = match role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "user",
                };
                json!({
                    "role": role_str,
                    "content": msg.content,
                })
            }
        })
        .flat_map(|v| {
            if let Some(parts) = v.get("_parts").and_then(Value::as_array) {
                parts.clone()
            } else {
                vec![v]
            }
        })
        .collect();
    Value::Array(items)
}

/// Normalize router (Anthropic), OpenAI nested, and Responses-native tool shapes
/// into Responses `tools` entries.
pub(super) fn convert_tools(tools_json: &Value) -> Vec<Value> {
    let Some(arr) = tools_json.as_array() else {
        return Vec::new();
    };
    arr.iter().filter_map(convert_one_tool).collect()
}

fn convert_one_tool(tool: &Value) -> Option<Value> {
    // Responses-native function: {type:"function", name, ...}
    if tool.get("type").and_then(Value::as_str) == Some("function")
        && tool.get("name").and_then(Value::as_str).is_some()
    {
        return Some(tool.clone());
    }

    // OpenAI chat-completions shape: {type:"function", function:{name,...}}
    if let Some(func) = tool.get("function").filter(|f| f.is_object()) {
        let name = func.get("name")?.as_str()?;
        let description = func
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let parameters = func
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        return Some(json!({
            "type": "function",
            "name": name,
            "description": description,
            "parameters": parameters,
        }));
    }

    // Built-in tool types (web_search, etc.) — no top-level name.
    if let Some(ty) = tool.get("type").and_then(Value::as_str)
        && tool.get("name").is_none()
        && BUILTIN_TOOL_TYPES.contains(&ty)
    {
        return Some(tool.clone());
    }

    // Router / Anthropic shape: {name, description, input_schema|parameters}
    let name = tool.get("name")?.as_str()?;
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let parameters = tool
        .get("parameters")
        .or_else(|| tool.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
    Some(json!({
        "type": "function",
        "name": name,
        "description": description,
        "parameters": parameters,
    }))
}

pub(super) fn parse_output(output: &[Value]) -> (String, Option<String>, Vec<ParsedToolCall>) {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();

    for item in output {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
        match item_type {
            "message" => {
                if let Some(parts) = item.get("content").and_then(Value::as_array) {
                    for part in parts {
                        if part.get("type").and_then(Value::as_str) == Some("output_text")
                            && let Some(text) = part.get("text").and_then(Value::as_str)
                        {
                            content.push_str(text);
                        }
                    }
                }
            }
            "function_call" => {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}")
                    .to_string();
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                tool_calls.push(ParsedToolCall {
                    tool_name: name,
                    arguments: serde_json::from_str(&arguments).unwrap_or_else(|_| json!({})),
                    call_id,
                });
            }
            "reasoning" => {
                if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                    for s in summary {
                        if let Some(text) = s.get("text").and_then(Value::as_str) {
                            if !reasoning.is_empty() {
                                reasoning.push('\n');
                            }
                            reasoning.push_str(text);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let reasoning = if reasoning.is_empty() {
        None
    } else {
        Some(reasoning)
    };
    (content, reasoning, tool_calls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, Role};

    #[test]
    fn convert_input_user_and_system() {
        let msgs = vec![Message::system("sys"), Message::user("hello")];
        let input = convert_input(&msgs);
        let arr = input.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["role"], "system");
        assert_eq!(arr[1]["content"], "hello");
    }

    #[test]
    fn convert_input_tool_result() {
        let mut msg = Message::user("result");
        msg.role = Role::Tool;
        msg.tool_call_id = Some("call_1".to_string());
        msg.content = r#"{"ok":true}"#.to_string();
        let input = convert_input(&[msg]);
        let item = &input.as_array().unwrap()[0];
        assert_eq!(item["type"], "function_call_output");
        assert_eq!(item["call_id"], "call_1");
    }

    #[test]
    fn convert_tools_from_router_shape() {
        let tools = json!([{
            "name": "get_time",
            "description": "time",
            "parameters": {"type": "object", "properties": {}}
        }]);
        let out = convert_tools(&tools);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "function");
        assert_eq!(out[0]["name"], "get_time");
    }

    #[test]
    fn convert_tools_from_openai_nested_shape() {
        let tools = json!([{
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "weather",
                "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
            }
        }]);
        let out = convert_tools(&tools);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "function");
        assert_eq!(out[0]["name"], "get_weather");
        assert_eq!(out[0]["description"], "weather");
        assert!(out[0]["parameters"]["properties"]["city"].is_object());
    }

    #[test]
    fn convert_tools_passthrough_builtin() {
        let tools = json!([{"type": "web_search"}]);
        let out = convert_tools(&tools);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "web_search");
    }

    #[test]
    fn convert_tools_rejects_unknown_type_without_name() {
        // OpenAI-shaped without nested function, and not a known builtin —
        // must not passthrough raw garbage.
        let tools = json!([{"type": "function"}]);
        let out = convert_tools(&tools);
        assert!(out.is_empty());
    }

    #[test]
    fn parse_output_message_and_function_call() {
        let output = vec![
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hi"}]
            }),
            json!({
                "type": "function_call",
                "call_id": "call_1",
                "name": "get_time",
                "arguments": "{}"
            }),
            json!({
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "think"}]
            }),
        ];
        let (content, reasoning, tools) = parse_output(&output);
        assert_eq!(content, "Hi");
        assert_eq!(reasoning.as_deref(), Some("think"));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "get_time");
        assert_eq!(tools[0].call_id.as_deref(), Some("call_1"));
    }
}
