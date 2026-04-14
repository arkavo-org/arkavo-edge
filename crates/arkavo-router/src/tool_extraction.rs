use arkavo_llm::{ParsedToolCall, ToolParser};

/// Extract keywords from task description for tool search
pub(crate) fn extract_keywords(task: &str) -> String {
    let words: Vec<&str> = task
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .filter(|w| {
            ![
                "this", "that", "with", "have", "from", "what", "where", "the", "and", "for",
            ]
            .contains(w)
        })
        .collect();
    words.join(" ")
}

/// Estimate token count from text (approximately 4 chars per token)
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Determine detail level based on model context size
pub(crate) fn detail_level_for_model(
    model: &crate::decision::ModelChoice,
) -> arkavo_mcp_tools::DetailLevel {
    use crate::decision::ModelChoice;
    match model {
        ModelChoice::LocalQwen3 | ModelChoice::LocalGemma270M => {
            arkavo_mcp_tools::DetailLevel::NameOnly
        }
        ModelChoice::LocalGemma4E2B
        | ModelChoice::LocalMinistral3B
        | ModelChoice::LocalGemma4B
        | ModelChoice::LocalGemma12B
        | ModelChoice::LocalDeepSeekCoder => arkavo_mcp_tools::DetailLevel::NameAndDescription,
        ModelChoice::LocalGemma4E4B
        | ModelChoice::LocalGemma4_26B
        | ModelChoice::LocalGemma4_31B
        | ModelChoice::LocalMinistral8B
        | ModelChoice::LocalQwen35_9B
        | ModelChoice::LocalQwen35_27B
        | ModelChoice::LocalGlm47Flash
        | ModelChoice::GeminiFlash
        | ModelChoice::GeminiPro
        | ModelChoice::ClaudeSonnet
        | ModelChoice::ClaudeOpus
        | ModelChoice::DeepSeekV32
        | ModelChoice::DeepSeekV32Speciale
        | ModelChoice::KimiK2 => arkavo_mcp_tools::DetailLevel::FullSchema,
    }
}

/// Search tools using hybrid approach: semantic (if available) + token-based
pub(crate) async fn search_tools_hybrid(
    registry: &arkavo_mcp_tools::ToolRegistry,
    query: &str,
    detail: arkavo_mcp_tools::DetailLevel,
    input_tokens: Option<usize>,
) -> Vec<arkavo_mcp_tools::MinimalToolInfo> {
    let augmented_query = if let Some(tokens) = input_tokens {
        const RLM_THRESHOLD: usize = 5600;
        if tokens > RLM_THRESHOLD {
            tracing::debug!(
                "Large context detected ({} tokens > {}), surfacing context tools",
                tokens,
                RLM_THRESHOLD
            );
            format!("{query} context")
        } else {
            query.to_string()
        }
    } else {
        query.to_string()
    };

    registry.search_tools(&augmented_query, detail)
}

/// Known programming language identifiers that aren't tool names
const LANG_IDENTIFIERS: &[&str] = &[
    "python",
    "py",
    "javascript",
    "js",
    "typescript",
    "ts",
    "rust",
    "go",
    "java",
    "ruby",
    "bash",
    "sh",
    "shell",
    "zsh",
    "powershell",
    "ps1",
    "sql",
    "json",
    "yaml",
    "toml",
];

/// Filter and extract tool calls from provider-returned tool_calls.
///
/// When a local LLM returns a fence like ```python\ncontext_search(...)```,
/// the provider parses this as tool_name="python" with the code as arguments.
/// This function detects language identifiers and extracts the nested tool calls.
pub fn filter_and_extract_tool_calls(tool_calls: Vec<ParsedToolCall>) -> Vec<ParsedToolCall> {
    tool_calls
        .into_iter()
        .flat_map(|call| {
            let tool_lower = call.tool_name.to_lowercase();

            if LANG_IDENTIFIERS.contains(&tool_lower.as_str()) {
                let content = match serde_json::to_string(&call.arguments) {
                    Ok(s) => s
                        .trim_matches('"')
                        .replace("\\n", "\n")
                        .replace("\\\"", "\""),
                    Err(e) => {
                        tracing::warn!(
                            "Failed to serialize arguments for language fence '{}': {e}",
                            call.tool_name
                        );
                        return vec![];
                    }
                };

                tracing::debug!(
                    "Found language fence '{}', extracting nested calls from: {}",
                    call.tool_name,
                    &content[..std::cmp::min(100, content.len())]
                );

                if let Some(extracted) = extract_python_function_calls(&content) {
                    return extracted;
                }
                if let Some(extracted) = extract_json_tool_calls(&content) {
                    return extracted;
                }

                tracing::debug!(
                    "No tool calls extracted from language fence '{}'",
                    call.tool_name
                );
                vec![]
            } else {
                vec![call]
            }
        })
        .collect()
}

/// Extract tool calls from text content using multiple parsing strategies
pub fn extract_tool_calls_from_text(content: &str) -> Vec<ParsedToolCall> {
    if let Ok(calls) = ToolParser::parse_fence(content) {
        let filtered: Vec<_> = calls
            .into_iter()
            .flat_map(|call| {
                let tool_lower = call.tool_name.to_lowercase();

                if tool_lower == "json" {
                    let json_str = match serde_json::to_string(&call.arguments) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("Failed to serialize JSON fence arguments: {e}");
                            return vec![];
                        }
                    };
                    if let Some(extracted) = extract_json_tool_calls(&json_str) {
                        return extracted;
                    }
                    // JSON fence without tool_name/tool/name fields is formatted
                    // output, not a tool call — drop it instead of returning
                    // a call with tool_name="json".
                    tracing::debug!("JSON fence did not contain tool call wrapper, skipping");
                    vec![]
                } else if LANG_IDENTIFIERS.contains(&tool_lower.as_str()) {
                    let fence_content = match serde_json::to_string(&call.arguments) {
                        Ok(s) => s.trim_matches('"').to_string(),
                        Err(e) => {
                            tracing::warn!(
                                "Failed to serialize language fence '{}' arguments: {e}",
                                call.tool_name
                            );
                            return vec![];
                        }
                    };
                    if let Some(extracted) = extract_python_function_calls(&fence_content) {
                        return extracted;
                    }
                    if let Some(extracted) = extract_tool_from_first_word(&fence_content) {
                        return extracted;
                    }
                    vec![]
                } else {
                    vec![call]
                }
            })
            .collect();

        if !filtered.is_empty() {
            return filtered;
        }
    }

    if let Some(calls) = extract_json_tool_calls(content) {
        return calls;
    }
    if let Ok(calls) = ToolParser::parse_xml(content) {
        return calls;
    }
    if let Some(calls) = extract_python_function_calls(content) {
        return calls;
    }
    if let Some(calls) = extract_curly_brace_tool_calls(content) {
        return calls;
    }

    Vec::new()
}

/// Extract `tool_name{json}` or `tool_name{}` format (Ministral/Mistral models)
///
/// Handles nested JSON by counting brace depth, and supports namespaced tool
/// names with colons (e.g. `game-rl:step{...}`).
fn extract_curly_brace_tool_calls(content: &str) -> Option<Vec<ParsedToolCall>> {
    let re = regex::Regex::new(r"([a-zA-Z][a-zA-Z0-9_\-:]*)\{").ok()?;
    let mut calls = Vec::new();

    for mat in re.find_iter(content) {
        let full = mat.as_str();
        let name = &full[..full.len() - 1]; // strip trailing '{'

        // Skip common non-tool patterns
        if name.len() <= 2 || ["if", "for", "while", "match", "fn", "let", "var"].contains(&name) {
            continue;
        }

        // Extract balanced JSON starting from the '{' at mat.end()-1
        let brace_start = mat.end() - 1;
        let bytes = content.as_bytes();
        let mut depth = 0i32;
        let mut end = brace_start;
        let mut in_string = false;
        let mut escape = false;

        for (i, &b) in bytes.iter().enumerate().skip(brace_start) {
            if escape {
                escape = false;
                continue;
            }
            if b == b'\\' && in_string {
                escape = true;
                continue;
            }
            if b == b'"' {
                in_string = !in_string;
                continue;
            }
            if in_string {
                continue;
            }
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if depth != 0 {
            continue;
        }

        let json_str = &content[brace_start..=end];
        let args: serde_json::Value = serde_json::from_str(json_str)
            .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

        // Gemma 4 uses "call:tool_name{...}" format — strip the "call:" prefix
        let clean_name = name.strip_prefix("call:").unwrap_or(name);
        calls.push(ParsedToolCall {
            tool_name: clean_name.to_string(),
            arguments: args,
            call_id: None,
        });
    }

    if calls.is_empty() { None } else { Some(calls) }
}

/// Extract Python-style function calls from text
pub(crate) fn extract_python_function_calls(content: &str) -> Option<Vec<ParsedToolCall>> {
    use regex::Regex;

    let content_to_parse = extract_code_block_content(content);

    let re =
        match Regex::new(r"(?:\w+\s*=\s*)?([a-zA-Z][a-zA-Z0-9_-]*)\(([^)]*(?:\[[^\]]*\][^)]*)*)\)")
        {
            Ok(r) => r,
            Err(_) => return None,
        };

    let mut calls = Vec::new();

    for cap in re.captures_iter(&content_to_parse) {
        let (tool_name, args_str) = match (cap.get(1), cap.get(2)) {
            (Some(t), Some(a)) => (t.as_str().to_string(), a.as_str()),
            _ => continue,
        };

        if tool_name.len() <= 2
            || [
                "print",
                "len",
                "str",
                "int",
                "float",
                "list",
                "dict",
                "range",
                "type",
                "python",
                "bash",
                "shell",
                "rust",
                "javascript",
                "js",
                "typescript",
                "ts",
                "json",
                "yaml",
            ]
            .contains(&tool_name.as_str())
        {
            continue;
        }

        let args = parse_python_kwargs(args_str);

        calls.push(ParsedToolCall {
            tool_name,
            arguments: serde_json::Value::Object(args),
            call_id: None,
        });
    }

    if calls.is_empty() { None } else { Some(calls) }
}

/// Extract content from markdown code blocks
pub(crate) fn extract_code_block_content(content: &str) -> String {
    use regex::Regex;

    let code_block_re = match Regex::new(r"```(?:\w+)?\s*\n([\s\S]*?)```") {
        Ok(r) => r,
        Err(_) => return content.to_string(),
    };

    let mut extracted = String::new();

    for cap in code_block_re.captures_iter(content) {
        if let Some(code) = cap.get(1) {
            extracted.push_str(code.as_str());
            extracted.push('\n');
        }
    }

    let without_blocks = code_block_re.replace_all(content, "");
    extracted.push_str(&without_blocks);

    extracted
}

/// Parse Python keyword arguments including lists
pub(crate) fn parse_python_kwargs(args_str: &str) -> serde_json::Map<String, serde_json::Value> {
    use regex::Regex;

    let mut args = serde_json::Map::new();

    let kwarg_re = match Regex::new(
        r#"(\w+)\s*=\s*(?:"([^"]*)"|'([^']*)'|(\d+(?:\.\d+)?)|(\[[^\]]*\])|(\w+))"#,
    ) {
        Ok(r) => r,
        Err(_) => return args,
    };

    for kwarg in kwarg_re.captures_iter(args_str) {
        let key = match kwarg.get(1) {
            Some(k) => k.as_str().to_string(),
            None => continue,
        };

        let value = if let Some(s) = kwarg.get(2) {
            serde_json::Value::String(s.as_str().to_string())
        } else if let Some(s) = kwarg.get(3) {
            serde_json::Value::String(s.as_str().to_string())
        } else if let Some(n) = kwarg.get(4) {
            if let Ok(num) = n.as_str().parse::<i64>() {
                serde_json::json!(num)
            } else if let Ok(num) = n.as_str().parse::<f64>() {
                serde_json::json!(num)
            } else {
                serde_json::Value::String(n.as_str().to_string())
            }
        } else if let Some(list_str) = kwarg.get(5) {
            parse_python_list(list_str.as_str())
        } else if let Some(id) = kwarg.get(6) {
            match id.as_str() {
                "True" => serde_json::Value::Bool(true),
                "False" => serde_json::Value::Bool(false),
                "None" => serde_json::Value::Null,
                other => serde_json::Value::String(other.to_string()),
            }
        } else {
            continue;
        };

        args.insert(key, value);
    }

    args
}

/// Parse a Python list literal like `[0, 1]` or `["a", "b"]`
pub(crate) fn parse_python_list(list_str: &str) -> serde_json::Value {
    let inner = list_str
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim();

    if inner.is_empty() {
        return serde_json::json!([]);
    }

    let mut items = Vec::new();

    for item in inner.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }

        if let Ok(n) = item.parse::<i64>() {
            items.push(serde_json::json!(n));
        } else if let Ok(n) = item.parse::<f64>() {
            items.push(serde_json::json!(n));
        } else if item.starts_with('"') && item.ends_with('"') {
            let s = item.trim_matches('"');
            items.push(serde_json::Value::String(s.to_string()));
        } else if item.starts_with('\'') && item.ends_with('\'') {
            let s = item.trim_matches('\'');
            items.push(serde_json::Value::String(s.to_string()));
        } else {
            items.push(serde_json::Value::String(item.to_string()));
        }
    }

    serde_json::Value::Array(items)
}

/// Extract tool name from first word on each line
fn extract_tool_from_first_word(content: &str) -> Option<Vec<ParsedToolCall>> {
    let mut calls = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        let first_word = trimmed
            .split(|c: char| c.is_whitespace() || c == '(' || c == '=')
            .next()
            .unwrap_or("")
            .trim();

        if first_word.is_empty()
            || !first_word.chars().next().unwrap_or(' ').is_alphabetic()
            || first_word
                .chars()
                .all(|c| c.is_lowercase() && c.is_ascii_alphabetic())
                && first_word.len() < 3
        {
            continue;
        }

        if [
            "import", "from", "def", "class", "if", "else", "for", "while", "return", "let",
            "const", "var", "function", "async", "await", "try", "catch", "finally",
        ]
        .contains(&first_word)
        {
            continue;
        }

        if trimmed.contains('(') || trimmed.contains('=') {
            calls.push(ParsedToolCall {
                tool_name: first_word.to_string(),
                arguments: serde_json::Value::Object(serde_json::Map::new()),
                call_id: None,
            });
        }
    }

    if calls.is_empty() { None } else { Some(calls) }
}

/// Extract JSON-formatted tool calls from text
fn extract_json_tool_calls(content: &str) -> Option<Vec<ParsedToolCall>> {
    let mut calls = Vec::new();

    let mut depth = 0i32;
    let mut start = None;

    for (i, c) in content.char_indices() {
        match c {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        let json_str = &content[s..=i];
                        if let Some(call) = parse_json_tool_call(json_str) {
                            calls.push(call);
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }

    if calls.is_empty() { None } else { Some(calls) }
}

/// Parse a single JSON object as a tool call
fn parse_json_tool_call(json_str: &str) -> Option<ParsedToolCall> {
    let json: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let obj = json.as_object()?;

    let tool_name = obj
        .get("tool")
        .or_else(|| obj.get("tool_name"))
        .or_else(|| obj.get("name"))
        .and_then(|v| v.as_str())?
        .to_string();

    if tool_name.is_empty() {
        return None;
    }

    let arguments = obj
        .get("parameters")
        .or_else(|| obj.get("args"))
        .or_else(|| obj.get("arguments"))
        .or_else(|| obj.get("input"))
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));

    Some(ParsedToolCall {
        tool_name,
        arguments,
        call_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_python_list_numbers() {
        let result = parse_python_list("[0, 1, 2]");
        assert_eq!(result, serde_json::json!([0, 1, 2]));
    }

    #[test]
    fn test_parse_python_list_strings() {
        let result = parse_python_list(r#"["security", "password"]"#);
        assert_eq!(result, serde_json::json!(["security", "password"]));
    }

    #[test]
    fn test_parse_python_kwargs_with_list() {
        let result = parse_python_kwargs(r#"indices=[0, 1], name="test""#);
        assert_eq!(result.get("indices"), Some(&serde_json::json!([0, 1])));
        assert_eq!(result.get("name"), Some(&serde_json::json!("test")));
    }

    #[test]
    fn test_extract_python_function_calls_with_list() {
        let content = r#"context_probe(indices=[0, 1])"#;
        let calls = extract_python_function_calls(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "context_probe");
        assert_eq!(
            calls[0].arguments.get("indices"),
            Some(&serde_json::json!([0, 1]))
        );
    }

    #[test]
    fn test_extract_python_function_calls_context_search() {
        let content = r#"context_search(keywords=["security", "auth"])"#;
        let calls = extract_python_function_calls(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "context_search");
        assert_eq!(
            calls[0].arguments.get("keywords"),
            Some(&serde_json::json!(["security", "auth"]))
        );
    }

    #[test]
    fn test_extract_tool_calls_from_llm_output() {
        let content = r#"
Here's how to search:

```python
results = context_search(keywords=["security", "password"])
chunks = context_probe(indices=[0, 1, 2])
```
"#;
        let calls = extract_tool_calls_from_text(content);
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().any(|c| c.tool_name == "context_search"));
        assert!(calls.iter().any(|c| c.tool_name == "context_probe"));
        assert!(!calls.iter().any(|c| c.tool_name == "python"));
    }

    #[test]
    fn test_extract_code_block_content() {
        let content = r#"
Let me explain:

```python
result = context_search(keywords=["auth"])
```

You can also use:

```bash
echo "hello"
```
"#;
        let extracted = extract_code_block_content(content);
        assert!(extracted.contains("result = context_search"));
    }

    #[test]
    fn test_curly_brace_tool_call_no_args() {
        let calls = extract_curly_brace_tool_calls("get_agent_time{}").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "get_agent_time");
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn test_curly_brace_tool_call_with_args() {
        let calls = extract_curly_brace_tool_calls(r#"get_time{"timezone": "UTC"}"#).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "get_time");
        assert_eq!(calls[0].arguments, serde_json::json!({"timezone": "UTC"}));
    }

    #[test]
    fn test_curly_brace_skips_keywords() {
        assert!(extract_curly_brace_tool_calls("if{}").is_none());
        assert!(extract_curly_brace_tool_calls("fn{}").is_none());
    }

    #[test]
    fn test_curly_brace_namespaced_nested_json() {
        let input = r#"game-rl:step{"AgentId": "player1", "Action": {"Type": "SetWorkPriority", "ColonistId": "Sugar", "WorkType": "Doctor", "Priority": 1}}"#;
        let calls = extract_curly_brace_tool_calls(input).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "game-rl:step");
        assert_eq!(calls[0].arguments["AgentId"], "player1");
        assert_eq!(calls[0].arguments["Action"]["Type"], "SetWorkPriority");
        assert_eq!(calls[0].arguments["Action"]["Priority"], 1);
    }

    #[test]
    fn test_json_fence_without_tool_wrapper_is_dropped() {
        // Model outputs ```json with raw data (no tool/tool_name/name field).
        // Should return empty, not a call with tool_name="json".
        let content = "```json\n{\"Action\":{\"Type\":\"DesignateHunt\",\"TargetId\":\"Deer123\"},\"AgentId\":\"player1\"}\n```";
        let calls = extract_tool_calls_from_text(content);
        assert!(
            calls.is_empty() || calls.iter().all(|c| c.tool_name != "json"),
            "Should not produce a tool call with name 'json', got: {:?}",
            calls.iter().map(|c| &c.tool_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_json_fence_with_tool_wrapper_extracts() {
        // Model outputs ```json with proper tool call wrapper — should extract.
        let content = "```json\n{\"tool\":\"game-rl:step\",\"parameters\":{\"Action\":{\"Type\":\"DesignateHunt\"}}}\n```";
        let calls = extract_tool_calls_from_text(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "game-rl:step");
    }
}
