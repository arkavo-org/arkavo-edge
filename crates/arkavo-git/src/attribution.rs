/// Attribution utilities for Arkavo Edge generated content
use std::fmt::Write as _;


const ARKAVO_EDGE_URL: &str = "https://arkavo.ai/edge";
const ARKAVO_EDGE_EMAIL: &str = "edge@arkavo.com";
const ARKAVO_EDGE_NAME: &str = "Arkavo Edge";

/// Formats a commit message with proper Arkavo Edge attribution
///
/// If `files_modified` is true, adds a Co-Authored-By trailer.
/// Always adds the "Generated with" footer.
pub fn format_commit_message(original_message: &str, files_modified: bool) -> String {
    let mut message = String::new();

    // Add the original message
    message.push_str(original_message.trim());

    // Ensure proper spacing before footers
    if !message.ends_with("\n\n") {
        if message.ends_with('\n') {
            message.push('\n');
        } else {
            message.push_str("\n\n");
        }
    }

    // Add the "Generated with" footer
    let _ = writeln!(
        message,
        "🤖 Generated with [Arkavo Edge]({ARKAVO_EDGE_URL})"
    );

    // Add Co-Authored-By if files were modified
    if files_modified {
        let _ = writeln!(
            message,
            "\nCo-Authored-By: {ARKAVO_EDGE_NAME} <{ARKAVO_EDGE_EMAIL}>"
        );
    }

    message
}

/// Formats a pull request body with Arkavo Edge attribution footer
pub fn format_pr_body(original_body: &str) -> String {
    let mut body = String::new();

    // Add the original body
    body.push_str(original_body.trim());

    // Ensure proper spacing before footer
    if !body.is_empty() && !body.ends_with("\n\n") {
        if body.ends_with('\n') {
            body.push('\n');
        } else {
            body.push_str("\n\n");
        }
    }

    // Add the PR footer
    let _ = writeln!(body, "🤖 Generated with [Arkavo Edge]({ARKAVO_EDGE_URL})");

    body
}

/// Checks if a message already contains Arkavo Edge attribution
pub fn has_attribution(message: &str) -> bool {
    message.contains("Generated with [Arkavo Edge]")
        || message.contains("Co-Authored-By: Arkavo Edge")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_commit_message_with_modifications() {
        let original = "Fix bug in parser";
        let result = format_commit_message(original, true);

        assert!(result.contains("Fix bug in parser"));
        assert!(result.contains("🤖 Generated with [Arkavo Edge]"));
        assert!(result.contains("Co-Authored-By: Arkavo Edge <edge@arkavo.com>"));
    }

    #[test]
    fn test_format_commit_message_without_modifications() {
        let original = "Update documentation";
        let result = format_commit_message(original, false);

        assert!(result.contains("Update documentation"));
        assert!(result.contains("🤖 Generated with [Arkavo Edge]"));
        assert!(!result.contains("Co-Authored-By"));
    }

    #[test]
    fn test_format_pr_body() {
        let original = "This PR adds a new feature.\n\n## Changes\n- Added X\n- Fixed Y";
        let result = format_pr_body(original);

        assert!(result.contains("This PR adds a new feature."));
        assert!(result.contains("🤖 Generated with [Arkavo Edge]"));
        assert!(!result.contains("Co-Authored-By"));
    }

    #[test]
    fn test_has_attribution() {
        assert!(has_attribution(
            "Some message\n\n🤖 Generated with [Arkavo Edge](https://arkavo.ai/edge)"
        ));
        assert!(has_attribution(
            "Message\n\nCo-Authored-By: Arkavo Edge <edge@arkavo.com>"
        ));
        assert!(!has_attribution("Regular commit message"));
    }

    #[test]
    fn test_spacing_handling() {
        // Test various input formats
        let test_cases = vec![
            ("No newline", false),
            ("Single newline\n", false),
            ("Double newline\n\n", false),
            ("Triple newline\n\n\n", false),
        ];

        for (input, with_files) in test_cases {
            let result = format_commit_message(input, with_files);
            // Should have exactly two newlines before the footer
            assert!(result.contains("\n\n🤖 Generated with"));
        }
    }
}
