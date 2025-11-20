/// Parser for REQUEST_TOOL: protocol in LLM responses
///
/// This module enables LLMs to request tools by keywords when they're not
/// visible due to progressive disclosure or limited context windows.
/// Parse tool requests from LLM response text
///
/// Looks for patterns like "REQUEST_TOOL: <keyword>" and extracts the keywords.
///
/// # Examples
/// ```
/// use arkavo_router::tool_request_parser::parse_tool_requests;
///
/// let response = "I need to check the time. REQUEST_TOOL: time";
/// let keywords = parse_tool_requests(response);
/// assert_eq!(keywords, vec!["time"]);
///
/// let multi = "REQUEST_TOOL: github\nREQUEST_TOOL: filesystem";
/// let keywords = parse_tool_requests(multi);
/// assert_eq!(keywords, vec!["github", "filesystem"]);
/// ```
pub fn parse_tool_requests(response: &str) -> Vec<String> {
    let mut keywords = Vec::new();

    // Match REQUEST_TOOL: followed by one or more word characters
    // Case-insensitive to handle variations
    for line in response.lines() {
        if let Some(keyword_start) = line.to_uppercase().find("REQUEST_TOOL:") {
            let after_marker = &line[keyword_start + 13..]; // Skip "REQUEST_TOOL:"

            // Extract the first word after the marker
            if let Some(keyword) = after_marker
                .split_whitespace()
                .next()
                .filter(|s| !s.is_empty())
            {
                keywords.push(keyword.to_lowercase());
            }
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
        let response = "I need help with time. REQUEST_TOOL: time";
        assert_eq!(parse_tool_requests(response), vec!["time"]);
    }

    #[test]
    fn test_multiple_requests() {
        let response = "REQUEST_TOOL: github\nREQUEST_TOOL: filesystem";
        assert_eq!(parse_tool_requests(response), vec!["github", "filesystem"]);
    }

    #[test]
    fn test_case_insensitive() {
        let response = "request_tool: Time\nREQUEST_TOOL: GITHUB";
        assert_eq!(parse_tool_requests(response), vec!["time", "github"]);
    }

    #[test]
    fn test_with_extra_text() {
        let response = "I'm not sure. REQUEST_TOOL: time for current datetime.";
        assert_eq!(parse_tool_requests(response), vec!["time"]);
    }

    #[test]
    fn test_no_requests() {
        let response = "I can help with that using existing tools.";
        assert_eq!(parse_tool_requests(response), Vec::<String>::new());
    }

    #[test]
    fn test_duplicate_requests() {
        let response = "REQUEST_TOOL: time\nREQUEST_TOOL: time";
        assert_eq!(parse_tool_requests(response), vec!["time"]);
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(parse_tool_requests(""), Vec::<String>::new());
    }

    #[test]
    fn test_mixed_content() {
        let response = "Let me help. REQUEST_TOOL: github for repos.\nAlso REQUEST_TOOL: filesystem for files.";
        assert_eq!(parse_tool_requests(response), vec!["github", "filesystem"]);
    }
}
