//! Bound tool output before retaining it in conversation history.

pub const MAX_TOOL_RESULT_BYTES: usize = 200_000;

/// Tool output is untrusted and may exceed the next provider's context window.
/// The marker is included in the byte allowance, with UTF-8 boundaries preserved.
pub fn bounded_tool_output(mut output: String) -> String {
    const MARKER: &str = "\n[OUTPUT TRUNCATED - result too large for LLM context]";
    if output.len() > MAX_TOOL_RESULT_BYTES {
        let mut end = MAX_TOOL_RESULT_BYTES - MARKER.len();
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str(MARKER);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_unicode_output_including_marker() {
        let output = bounded_tool_output("界".repeat(MAX_TOOL_RESULT_BYTES));
        assert!(output.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(output.ends_with("[OUTPUT TRUNCATED - result too large for LLM context]"));
        assert!(output.starts_with("界"));
    }

    #[test]
    fn preserves_small_output() {
        assert_eq!(bounded_tool_output("{\"ok\":true}".into()), "{\"ok\":true}");
    }
}
