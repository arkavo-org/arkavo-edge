//! Response processing functions
//!
//! Functions for cleaning LLM response output by stripping internal
//! blocks and conversation prefixes.

use std::sync::atomic::{AtomicBool, Ordering};

/// Debug flag for response processing
static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

/// Set debug mode
pub fn set_debug(enabled: bool) {
    SHOW_DEBUG.store(enabled, Ordering::SeqCst);
}

/// Check if debug mode is enabled
pub fn is_debug() -> bool {
    SHOW_DEBUG.load(Ordering::Relaxed)
}

/// Strip `<think>...</think>` blocks from content
///
/// In debug mode, prints thinking content to stderr before stripping.
/// Handles both closed and unclosed think blocks.
pub fn strip_think_blocks(content: &str) -> String {
    let mut result = content.to_string();
    let debug = is_debug();

    // Extract and optionally display think blocks
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result[start..].find("</think>").map(|e| start + e) {
            let think_content = &result[start + 7..end];
            if debug && !think_content.trim().is_empty() {
                eprintln!("[thinking] {}", think_content.trim());
            }
            let end_pos = end + "</think>".len();
            result = format!("{}{}", &result[..start], &result[end_pos..]);
        } else {
            // Unclosed think block - remove from <think> to end
            if debug {
                let think_content = &result[start + 7..];
                if !think_content.trim().is_empty() {
                    eprintln!("[thinking] {}", think_content.trim());
                }
            }
            result = result[..start].to_string();
            break;
        }
    }

    result
}

/// Strip `<tool>...</tool>` blocks from content
///
/// These blocks represent tool executions that have already been processed.
/// Handles both closed and unclosed tool blocks.
pub fn strip_tool_blocks(content: &str) -> String {
    let mut result = content.to_string();

    while let Some(start) = result.find("<tool>") {
        if let Some(end) = result.find("</tool>") {
            let end_pos = end + "</tool>".len();
            result = format!("{}{}", &result[..start], &result[end_pos..]);
        } else {
            // Unclosed tool block - remove from <tool> to end
            result = result[..start].to_string();
            break;
        }
    }

    result
}

/// Strip conversation prefixes from content
///
/// Removes prefixes like "Human:", "Assistant:", "User:", etc.
/// that shouldn't appear in final output.
pub fn strip_conversation_prefixes(content: &str) -> String {
    let mut result = content.to_string();

    for prefix in ["Human:", "Assistant:", "User:", "A:", "Q:"] {
        result = result.replace(prefix, "");
    }

    result
}

/// Full sanitization pipeline for LLM responses
///
/// Combines all processing steps:
/// 1. Strip think blocks
/// 2. Strip tool blocks
/// 3. Strip conversation prefixes
/// 4. Clean up whitespace
pub fn sanitize_response(content: &str) -> String {
    let mut result = strip_think_blocks(content);
    result = strip_tool_blocks(&result);
    result = strip_conversation_prefixes(&result);

    // Clean up excessive whitespace
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_think_blocks() {
        let content = "Let me think...<think>internal reasoning</think>The answer is 4.";
        let stripped = strip_think_blocks(content);
        assert!(!stripped.contains("<think>"));
        assert!(!stripped.contains("internal reasoning"));
        assert!(stripped.contains("The answer is 4"));
    }

    #[test]
    fn test_strip_unclosed_think_block() {
        let content = "Start<think>never closed...";
        let stripped = strip_think_blocks(content);
        assert!(!stripped.contains("<think>"));
        assert!(stripped.contains("Start"));
        assert!(!stripped.contains("never closed"));
    }

    #[test]
    fn test_strip_tool_blocks() {
        let content = "Result: <tool>execute(x=1)</tool>Done.";
        let stripped = strip_tool_blocks(content);
        assert!(!stripped.contains("<tool>"));
        assert!(stripped.contains("Result:"));
        assert!(stripped.contains("Done."));
    }

    #[test]
    fn test_strip_unclosed_tool_block() {
        let content = "Start<tool>never closed";
        let stripped = strip_tool_blocks(content);
        assert!(!stripped.contains("<tool>"));
        assert!(stripped.contains("Start"));
    }

    #[test]
    fn test_sanitize_response_removes_prefixes() {
        let content = "Human: hi\nAssistant: hello";
        let sanitized = sanitize_response(content);
        assert!(!sanitized.contains("Human:"));
        assert!(!sanitized.contains("Assistant:"));
    }

    #[test]
    fn test_sanitize_response_full() {
        let content = "<think>reasoning</think>Human: ignored\nThe answer is 42.";
        let sanitized = sanitize_response(content);
        assert!(!sanitized.contains("<think>"));
        assert!(!sanitized.contains("Human:"));
        assert!(sanitized.contains("42"));
    }

    #[test]
    fn test_sanitize_response_cleans_whitespace() {
        let content = "Line 1\n\n\n\nLine 2";
        let sanitized = sanitize_response(content);
        assert!(!sanitized.contains("\n\n\n"));
    }

    #[test]
    fn test_multiple_think_blocks() {
        let content = "<think>first</think>middle<think>second</think>end";
        let stripped = strip_think_blocks(content);
        assert!(!stripped.contains("<think>"));
        assert!(stripped.contains("middle"));
        assert!(stripped.contains("end"));
    }

    #[test]
    fn test_nested_tool_blocks() {
        let content = "before<tool>inner</tool>between<tool>inner2</tool>after";
        let stripped = strip_tool_blocks(content);
        assert!(!stripped.contains("<tool>"));
        assert!(stripped.contains("before"));
        assert!(stripped.contains("between"));
        assert!(stripped.contains("after"));
    }

    #[test]
    fn test_debug_flag() {
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }
}
