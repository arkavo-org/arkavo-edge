//! Response quality judge for task results
//!
//! Scores agent responses to detect low-quality outputs: empty responses,
//! generic non-answers, tool hallucinations, and output loops. Feeds quality
//! scores back into Thompson Sampling so the mesh learns to route around
//! agents that produce poor results.

/// Quality judgment for a task result
#[derive(Debug, Clone)]
pub struct TaskJudgment {
    /// Quality score from 0.0 (worthless) to 1.0 (excellent)
    pub quality_score: f64,
    /// Human-readable issues detected
    pub issues: Vec<String>,
}

/// Judge a task result against its description
///
/// Examines the response text for quality signals:
/// - Empty or trivially short responses
/// - Generic non-answers ("Task completed", "Done")
/// - Tool error markers from failed tool calls
/// - Output loops (repeated content)
/// - Response substance relative to task complexity
pub fn judge(task_description: &str, result: &str) -> TaskJudgment {
    let mut score = 1.0;
    let mut issues = Vec::new();

    // Empty response = total failure
    if result.is_empty() || result.trim().is_empty() {
        return TaskJudgment {
            quality_score: 0.0,
            issues: vec!["Empty response".into()],
        };
    }

    let trimmed = result.trim();
    let lower = trimmed.to_lowercase();

    // Generic non-answer detection
    if is_generic_response(&lower) {
        issues.push("Generic response with no substantive content".into());
        score -= 0.8;
    }

    // Response length vs task complexity
    let task_words = task_description.split_whitespace().count();
    let result_words = trimmed.split_whitespace().count();
    if task_words > 5 && result_words < 10 {
        issues.push(format!(
            "Response too short: {} words for a {}-word task",
            result_words, task_words
        ));
        score -= 0.3;
    }

    // Tool error markers in response
    let (tool_errors, tool_total) = count_tool_results(trimmed);
    if tool_errors > 0 {
        let error_ratio = tool_errors as f64 / tool_total as f64;
        issues.push(format!(
            "{}/{} tool calls failed ({:.0}% failure rate)",
            tool_errors,
            tool_total,
            error_ratio * 100.0
        ));
        // Scale penalty by error ratio: all failures = -0.6, half = -0.3
        score -= error_ratio * 0.6;
    }

    // Tool hallucination detection (Rust stdlib methods as tool names)
    let hallucination_count = count_hallucinated_tools(trimmed);
    if hallucination_count > 0 {
        issues.push(format!(
            "{} tool hallucinations (non-existent tools called)",
            hallucination_count
        ));
        score -= (hallucination_count as f64 * 0.1).min(0.4);
    }

    // Output loop detection
    if has_repetition(trimmed) {
        issues.push("Output loop detected (repeated content)".into());
        score -= 0.4;
    }

    TaskJudgment {
        quality_score: score.clamp(0.0, 1.0),
        issues,
    }
}

/// Detect generic non-answer responses
fn is_generic_response(lower: &str) -> bool {
    let generic_patterns = [
        "task completed",
        "task complete",
        "done",
        "ok",
        "completed successfully",
        "i have completed",
        "the task has been completed",
        "analysis complete",
    ];

    // Exact match or very close to a generic pattern
    generic_patterns
        .iter()
        .any(|p| lower == *p || (lower.len() < 30 && lower.contains(p)))
}

/// Count tool success/failure markers in result text
///
/// The conductor formats tool results as:
/// - `## Tool: name` (success)
/// - `## Tool: name (Error)` (failure)
fn count_tool_results(text: &str) -> (usize, usize) {
    let errors = text.matches("(Error)").count();
    let total = text.matches("## Tool:").count();
    (errors, total.max(errors))
}

/// Count hallucinated tool calls (Rust/Python stdlib methods used as tool names)
fn count_hallucinated_tools(text: &str) -> usize {
    let hallucination_patterns = [
        "Tool 'open' not found",
        "Tool 'unwrap' not found",
        "Tool 'exit' not found",
        "Tool 'iter' not found",
        "Tool 'sum' not found",
        "Tool 'save' not found",
        "Tool 'process' not found",
        "Tool 'new' not found",
        "Tool 'main' not found",
        "Tool 'read_to_string' not found",
        "Tool 'split_whitespace' not found",
        "Tool 'map_err' not found",
        "Tool 'parse_numbers' not found",
        "Tool 'read_file' not found",
        "not found in any MCP server",
    ];

    hallucination_patterns
        .iter()
        .filter(|p| text.contains(*p))
        .count()
}

/// Detect repetitive patterns indicating an output loop
fn has_repetition(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 5 {
        return false;
    }

    // Repeated consecutive lines
    let mut repeat_count = 1;
    for i in 1..lines.len() {
        if lines[i] == lines[i - 1] && !lines[i].trim().is_empty() {
            repeat_count += 1;
            if repeat_count >= 3 {
                return true;
            }
        } else {
            repeat_count = 1;
        }
    }

    // Repeated phrase chunks
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() > 20 {
        let chunk_size = 5;
        for i in 0..words.len().saturating_sub(chunk_size * 2) {
            let chunk1 = &words[i..i + chunk_size];
            let chunk2 = &words[i + chunk_size..i + chunk_size * 2];
            if chunk1 == chunk2 {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_response() {
        let j = judge("Analyze main.rs", "");
        assert_eq!(j.quality_score, 0.0);
        assert!(j.issues[0].contains("Empty"));
    }

    #[test]
    fn test_generic_task_completed() {
        let j = judge(
            "Analyze the main.rs file for code quality improvements",
            "Task completed",
        );
        assert!(
            j.quality_score < 0.3,
            "Generic response should score low: {}",
            j.quality_score
        );
        assert!(j.issues.iter().any(|i| i.contains("Generic")));
    }

    #[test]
    fn test_generic_done() {
        let j = judge("Generate unit tests", "done");
        assert!(j.quality_score < 0.3);
    }

    #[test]
    fn test_substantive_response() {
        let j = judge(
            "Analyze the main.rs file",
            "The main.rs file has several areas for improvement: \
             1. Error handling uses unwrap() in 3 places which should use proper Result propagation. \
             2. The configuration parsing could be extracted into a separate module. \
             3. There are unused imports on lines 5 and 12.",
        );
        assert!(
            j.quality_score > 0.8,
            "Substantive response should score high: {}",
            j.quality_score
        );
        assert!(j.issues.is_empty());
    }

    #[test]
    fn test_tool_errors_in_response() {
        let result = "## Tool: analyze\nGood results\n\n## Tool: lint (Error)\nTool 'lint' not found\n\n## Tool: format (Error)\nFailed";
        let j = judge("Check code quality", result);
        assert!(
            j.quality_score < 0.8,
            "Tool errors should degrade quality: {}",
            j.quality_score
        );
        assert!(j.issues.iter().any(|i| i.contains("tool calls failed")));
    }

    #[test]
    fn test_all_tools_failed() {
        let result = "## Tool: open (Error)\nNot found\n## Tool: read (Error)\nNot found\n## Tool: parse (Error)\nNot found";
        let j = judge("Analyze code", result);
        assert!(
            j.quality_score < 0.5,
            "All tools failing should score very low: {}",
            j.quality_score
        );
    }

    #[test]
    fn test_hallucinated_tools() {
        let result = "I tried to analyze but: Tool 'open' not found in any MCP server, Tool 'unwrap' not found in any MCP server";
        let j = judge("Analyze code", result);
        assert!(j.issues.iter().any(|i| i.contains("hallucination")));
    }

    #[test]
    fn test_output_loop() {
        let result = "The answer is 42\nThe answer is 42\nThe answer is 42\nThe answer is 42\nThe answer is 42";
        let j = judge("What is the meaning of life?", result);
        assert!(j.quality_score < 0.7);
        assert!(j.issues.iter().any(|i| i.contains("loop")));
    }

    #[test]
    fn test_short_response_for_complex_task() {
        let j = judge(
            "Analyze the authentication flow for security vulnerabilities and suggest improvements",
            "Looks fine",
        );
        assert!(
            j.quality_score < 0.7,
            "Short response for complex task: {}",
            j.quality_score
        );
        assert!(j.issues.iter().any(|i| i.contains("too short")));
    }

    #[test]
    fn test_perfect_score() {
        let j = judge("What is 2+2?", "The result of 2+2 is 4.");
        assert_eq!(j.quality_score, 1.0);
        assert!(j.issues.is_empty());
    }

    #[test]
    fn test_count_tool_results() {
        assert_eq!(count_tool_results("## Tool: a\n## Tool: b (Error)"), (1, 2));
        assert_eq!(count_tool_results("no tools here"), (0, 0));
        assert_eq!(
            count_tool_results("## Tool: a (Error)\n## Tool: b (Error)"),
            (2, 2)
        );
    }

    #[test]
    fn test_is_generic_response() {
        assert!(is_generic_response("task completed"));
        assert!(is_generic_response("done"));
        assert!(is_generic_response("ok"));
        assert!(!is_generic_response(
            "the main.rs file has several issues including error handling"
        ));
    }

    #[test]
    fn test_hallucination_detection() {
        assert_eq!(count_hallucinated_tools("Tool 'open' not found"), 1);
        assert_eq!(
            count_hallucinated_tools("Tool 'unwrap' not found, Tool 'exit' not found"),
            2
        );
        assert_eq!(count_hallucinated_tools("Everything worked fine"), 0);
    }

    #[test]
    fn test_repetition_detection() {
        assert!(has_repetition("a\na\na\na\na"));
        assert!(!has_repetition("line 1\nline 2\nline 3"));
        assert!(!has_repetition("short"));
    }
}
