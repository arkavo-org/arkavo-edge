use arkavo_mcp_tools::registry::ToolInfo;
use std::fmt::Write as _;

/// Generate a GBNF grammar that constrains output to valid fence-format tool calls.
///
/// The grammar allows free text followed by an optional tool call in fence format.
/// Tool names are constrained to actual registered tools, preventing hallucination.
///
/// Returns `(grammar_string, root_rule_name)`.
pub fn fence_grammar_for_tools(tools: &[ToolInfo]) -> (String, String) {
    let mut grammar = String::with_capacity(1024);

    // Root: free text optionally followed by a tool call
    grammar.push_str("root ::= free-text tool-call free-text\n");
    grammar.push_str("free-text ::= [^`]* | \"`\" [^`] free-text | \"`\" \"`\" [^`] free-text\n");

    // Tool call: ```tool_name\nparams\n```
    grammar.push_str("tool-call ::= \"```\" tool-name \"\\n\" params \"```\"\n");

    // Tool names: alternation of all registered tools
    if tools.is_empty() {
        grammar.push_str("tool-name ::= [a-z] [a-z0-9_-]*\n");
    } else {
        grammar.push_str("tool-name ::= ");
        for (i, tool) in tools.iter().enumerate() {
            if i > 0 {
                grammar.push_str(" | ");
            }
            let _ = write!(grammar, "\"{}\"", escape_gbnf(&tool.name));
        }
        grammar.push('\n');
    }

    // Parameters: zero or more key: value lines
    grammar.push_str("params ::= (param-line \"\\n\")*\n");
    grammar.push_str("param-line ::= param-key \": \" param-value\n");
    grammar.push_str("param-key ::= [a-zA-Z_] [a-zA-Z0-9_]*\n");
    grammar.push_str("param-value ::= [^\\n]+\n");

    (grammar, "root".to_string())
}

/// Generate a GBNF grammar specifically for the tool call portion only.
///
/// Used with lazy grammar patterns where the trigger is "```" — the grammar
/// only needs to constrain the content after the trigger.
pub fn fence_grammar_after_trigger(tools: &[ToolInfo]) -> (String, String) {
    let mut grammar = String::with_capacity(512);

    // Root starts right after "```" trigger
    grammar.push_str("root ::= tool-name \"\\n\" params \"```\" free-tail\n");

    if tools.is_empty() {
        grammar.push_str("tool-name ::= [a-z] [a-z0-9_-]*\n");
    } else {
        grammar.push_str("tool-name ::= ");
        for (i, tool) in tools.iter().enumerate() {
            if i > 0 {
                grammar.push_str(" | ");
            }
            let _ = write!(grammar, "\"{}\"", escape_gbnf(&tool.name));
        }
        grammar.push('\n');
    }

    grammar.push_str("params ::= (param-line \"\\n\")*\n");
    grammar.push_str("param-line ::= param-key \": \" param-value\n");
    grammar.push_str("param-key ::= [a-zA-Z_] [a-zA-Z0-9_]*\n");
    grammar.push_str("param-value ::= [^\\n`]+\n");
    // Allow trailing text after the closing fence
    grammar.push_str("free-tail ::= [^`]*\n");

    (grammar, "root".to_string())
}

/// The trigger patterns for lazy grammar activation.
/// When the model outputs "```", grammar enforcement kicks in.
pub fn fence_trigger_patterns() -> Vec<&'static str> {
    vec!["```"]
}

fn escape_gbnf(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_tool(name: &str) -> ToolInfo {
        ToolInfo {
            name: name.to_string(),
            category: "test".to_string(),
            description: format!("Test {name}"),
            schema: json!({"type": "object", "properties": {}}),
        }
    }

    #[test]
    fn test_fence_grammar_generates_valid_gbnf() {
        let tools = vec![test_tool("search"), test_tool("read_file")];
        let (grammar, root) = fence_grammar_for_tools(&tools);

        assert_eq!(root, "root");
        assert!(grammar.contains("\"search\""));
        assert!(grammar.contains("\"read_file\""));
        assert!(grammar.contains("tool-call"));
        assert!(grammar.contains("param-key"));
    }

    #[test]
    fn test_fence_grammar_empty_tools() {
        let (grammar, _) = fence_grammar_for_tools(&[]);
        assert!(grammar.contains("[a-z] [a-z0-9_-]*"));
    }

    #[test]
    fn test_fence_grammar_after_trigger() {
        let tools = vec![test_tool("get_weather")];
        let (grammar, root) = fence_grammar_after_trigger(&tools);

        assert_eq!(root, "root");
        assert!(grammar.contains("\"get_weather\""));
        assert!(!grammar.contains("free-text")); // No free text before tool
        assert!(grammar.contains("free-tail")); // Free text after
    }

    #[test]
    fn test_escape_gbnf() {
        assert_eq!(escape_gbnf("simple"), "simple");
        assert_eq!(escape_gbnf("has\"quote"), "has\\\"quote");
    }

    #[test]
    fn test_trigger_patterns() {
        let patterns = fence_trigger_patterns();
        assert!(patterns.contains(&"```"));
    }
}
