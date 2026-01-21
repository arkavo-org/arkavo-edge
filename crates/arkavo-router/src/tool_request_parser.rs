/// Parser for tool capability requests in LLM responses
///
/// This module enables LLMs to request tools using a natural annotation format
/// when tools aren't visible due to progressive disclosure.
///
/// Format: `[need: keyword]` - simple bracketed annotation
///
/// Parse tool requests from LLM response text
///
/// Looks for patterns like `[need: time]` or `[need: filesystem]` and extracts keywords.
///
/// # Examples
/// ```
/// use arkavo_router::tool_request_parser::parse_tool_requests;
///
/// let response = "I need to check the current time. [need: time]";
/// let keywords = parse_tool_requests(response);
/// assert_eq!(keywords, vec!["time"]);
///
/// let multi = "For this I'll need [need: github] and [need: filesystem] access.";
/// let keywords = parse_tool_requests(multi);
/// assert_eq!(keywords, vec!["github", "filesystem"]);
/// ```
pub fn parse_tool_requests(response: &str) -> Vec<String> {
    let mut keywords = Vec::new();

    // Match [need: keyword] pattern - case insensitive
    // Supports: [need: time], [NEED: github], [Need: filesystem]
    let mut remaining = response;
    while let Some(start) = remaining.to_lowercase().find("[need:") {
        let after_marker = &remaining[start + 6..]; // Skip "[need:"

        // Find the closing bracket
        if let Some(end) = after_marker.find(']') {
            let keyword = after_marker[..end].trim().to_lowercase();
            if !keyword.is_empty() {
                keywords.push(keyword);
            }
            remaining = &after_marker[end + 1..];
        } else {
            break;
        }
    }

    // Remove duplicates while preserving order
    let mut seen = std::collections::HashSet::new();
    keywords.retain(|k| seen.insert(k.clone()));

    keywords
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_request() {
        let response = "I need to check the time. [need: time]";
        assert_eq!(parse_tool_requests(response), vec!["time"]);
    }

    #[test]
    fn test_multiple_requests() {
        let response = "I'll need [need: github] and [need: filesystem] for this.";
        assert_eq!(parse_tool_requests(response), vec!["github", "filesystem"]);
    }

    #[test]
    fn test_case_insensitive() {
        let response = "[Need: Time] and [NEED: GITHUB]";
        assert_eq!(parse_tool_requests(response), vec!["time", "github"]);
    }

    #[test]
    fn test_inline() {
        let response = "To read files [need: filesystem] I would use the read tool.";
        assert_eq!(parse_tool_requests(response), vec!["filesystem"]);
    }

    #[test]
    fn test_no_requests() {
        let response = "I can help with that using the available tools.";
        assert_eq!(parse_tool_requests(response), Vec::<String>::new());
    }

    #[test]
    fn test_duplicate_requests() {
        let response = "[need: time] and also [need: time]";
        assert_eq!(parse_tool_requests(response), vec!["time"]);
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(parse_tool_requests(""), Vec::<String>::new());
    }

    #[test]
    fn test_multiword_keyword() {
        let response = "[need: code search]";
        assert_eq!(parse_tool_requests(response), vec!["code search"]);
    }

    #[test]
    fn test_no_closing_bracket() {
        let response = "[need: time without closing";
        assert_eq!(parse_tool_requests(response), Vec::<String>::new());
    }
}
