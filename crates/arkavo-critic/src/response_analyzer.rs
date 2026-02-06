//! Response analysis for detecting quality issues in LLM outputs
//!
//! Pure analysis logic with no persistence dependencies. Detects code fence
//! misuse, output loops, wrong expert routing, and timeout/empty responses.

use std::collections::HashSet;

/// Analyzer for detecting issues in model responses
pub struct ResponseAnalyzer {
    model_name: String,
    model_family: String,
}

/// Types of issues that can be detected in model responses
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedIssue {
    /// Model wrapped output in code fences when not needed
    UnwantedCodeFence,
    /// Model looped or repeated output
    OutputLoop,
    /// Model triggered wrong expert (e.g., coding for chat)
    WrongExpert(String),
    /// Response was empty or timed out
    Timeout,
}

/// Result of analyzing a response
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// The issue detected
    pub issue: DetectedIssue,
    /// Model family that produced the response
    pub model_family: String,
    /// Category string formatted as "model:{family}:{type}"
    pub category: String,
}

impl ResponseAnalyzer {
    /// Create a new analyzer for a specific model
    pub fn new(model_name: &str) -> Self {
        let model_family = detect_model_family(model_name);
        Self {
            model_name: model_name.to_string(),
            model_family,
        }
    }

    /// Get the detected model family
    pub fn model_family(&self) -> &str {
        &self.model_family
    }

    /// Get the model name
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Analyze a response and return an AnalysisResult if issues are detected
    pub fn analyze(&self, prompt: &str, response: &str) -> Option<AnalysisResult> {
        if let Some(issue) = self.detect_issue(prompt, response) {
            let category = issue_to_category(&issue, &self.model_family);
            Some(AnalysisResult {
                issue,
                model_family: self.model_family.clone(),
                category,
            })
        } else {
            None
        }
    }

    /// Detect specific issues in the response
    fn detect_issue(&self, prompt: &str, response: &str) -> Option<DetectedIssue> {
        let lower_prompt = prompt.to_lowercase();
        let lower_response = response.to_lowercase();

        // Check for timeout/empty
        if response.is_empty() || response.trim().is_empty() {
            return Some(DetectedIssue::Timeout);
        }

        // Check for unwanted code fences on simple queries
        if is_simple_query(&lower_prompt) && has_code_fence(response) {
            return Some(DetectedIssue::UnwantedCodeFence);
        }

        // Check for output loops (repetition)
        if has_repetition(response) {
            return Some(DetectedIssue::OutputLoop);
        }

        // Check for wrong expert routing (GLM-specific)
        if self.model_family == "glm"
            && is_chat_query(&lower_prompt)
            && has_code_indicators(&lower_response)
        {
            return Some(DetectedIssue::WrongExpert("conversation".to_string()));
        }

        None
    }
}

/// Check if the prompt is a simple query that shouldn't need code
pub fn is_simple_query(prompt: &str) -> bool {
    // Greetings
    if prompt.starts_with("hi") || prompt.starts_with("hello") || prompt.starts_with("hey") {
        return true;
    }

    // Simple math
    let math_patterns = ["what is", "calculate", "compute", "how much"];
    let contains_math = math_patterns.iter().any(|p| prompt.contains(p));
    let has_numbers = prompt.chars().any(|c| c.is_ascii_digit());
    if contains_math && has_numbers && prompt.len() < 50 {
        return true;
    }

    // Simple factual questions
    let factual = ["capital of", "who is", "when was", "where is"];
    if factual.iter().any(|p| prompt.contains(p)) {
        return true;
    }

    false
}

/// Check if response contains code fences
pub fn has_code_fence(response: &str) -> bool {
    response.contains("```")
}

/// Check for repetitive patterns indicating a loop
pub fn has_repetition(response: &str) -> bool {
    let lines: Vec<&str> = response.lines().collect();
    if lines.len() < 5 {
        return false;
    }

    // Check for repeated consecutive lines
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

    // Check for Q&A loop pattern (GLM often generates "What is X+Y?" sequences)
    let question_count = lines
        .iter()
        .filter(|l| l.contains("What is") || l.contains("what is"))
        .count();
    if question_count >= 3 {
        return true;
    }

    // Check for response significantly longer than expected
    if lines.len() > 10 {
        let short_lines = lines.iter().filter(|l| l.len() < 20).count();
        if short_lines > lines.len() / 2 {
            return true;
        }
    }

    // Check for repeated phrases
    let words: Vec<&str> = response.split_whitespace().collect();
    if words.len() > 20 {
        let chunk_size = 5;
        for i in 0..(words.len() - chunk_size * 2) {
            let chunk1 = &words[i..i + chunk_size];
            let chunk2 = &words[i + chunk_size..i + chunk_size * 2];
            if chunk1 == chunk2 {
                return true;
            }
        }
    }

    false
}

/// Check if this is a conversational query
pub fn is_chat_query(prompt: &str) -> bool {
    let chat_indicators = [
        "hi",
        "hello",
        "hey",
        "thanks",
        "thank you",
        "how are",
        "what's up",
        "tell me about",
        "explain",
    ];
    chat_indicators.iter().any(|p| prompt.contains(p))
}

/// Check if response has coding/tool expert indicators
pub fn has_code_indicators(response: &str) -> bool {
    response.contains("def ")
        || response.contains("import ")
        || response.contains("print(")
        || response.contains("```python")
        || response.contains("```")
}

/// Detect model family from name string
pub fn detect_model_family(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("glm") {
        "glm".to_string()
    } else if lower.contains("gemma") {
        "gemma".to_string()
    } else if lower.contains("qwen") {
        "qwen".to_string()
    } else if lower.contains("mistral") || lower.contains("ministral") {
        "mistral".to_string()
    } else if lower.contains("deepseek") {
        "deepseek".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Extract just the first meaningful answer from a potentially loopy response.
/// For math queries, this returns just the number on the first line.
pub fn extract_first_answer(response: &str, is_math_query: bool) -> String {
    let trimmed = response.trim();

    if trimmed.is_empty() {
        return String::new();
    }

    // For math queries, try to extract just the number
    if is_math_query {
        let first_line = trimmed.lines().next().unwrap_or(trimmed);
        let clean = first_line.trim();
        if clean
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == ' ')
        {
            return clean.to_string();
        }
        return first_line.to_string();
    }

    // For non-math, return everything before repetition starts
    let lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() <= 3 {
        return trimmed.to_string();
    }

    // Find where repetition starts
    let mut result_lines = Vec::new();
    let mut seen_patterns = HashSet::new();

    for line in lines {
        let normalized = line.trim().to_lowercase();
        if normalized.is_empty() {
            result_lines.push(line);
            continue;
        }
        if seen_patterns.contains(&normalized) {
            break;
        }
        seen_patterns.insert(normalized);
        result_lines.push(line);
    }

    result_lines.join("\n")
}

/// Check if a prompt is a simple math query
pub fn is_simple_math_query(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    lower.contains("what is")
        && (lower.contains('+')
            || lower.contains('-')
            || lower.contains('*')
            || lower.contains('/'))
        && prompt.len() < 50
}

/// Convert a DetectedIssue to a category string
pub fn issue_to_category(issue: &DetectedIssue, model_family: &str) -> String {
    let issue_name = match issue {
        DetectedIssue::UnwantedCodeFence => "code_fence",
        DetectedIssue::OutputLoop => "loop",
        DetectedIssue::WrongExpert(_) => "wrong_expert",
        DetectedIssue::Timeout => "timeout",
    };
    format!("model:{model_family}:{issue_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_model_family_glm() {
        assert_eq!(detect_model_family("glm-4.7-flash"), "glm");
    }

    #[test]
    fn test_detect_model_family_gemma() {
        assert_eq!(detect_model_family("gemma-2-9b"), "gemma");
    }

    #[test]
    fn test_detect_model_family_qwen() {
        assert_eq!(detect_model_family("qwen2.5-coder"), "qwen");
    }

    #[test]
    fn test_detect_model_family_mistral() {
        assert_eq!(detect_model_family("mistral-7b"), "mistral");
        assert_eq!(detect_model_family("ministral-3b"), "mistral");
    }

    #[test]
    fn test_detect_model_family_deepseek() {
        assert_eq!(detect_model_family("deepseek-r1"), "deepseek");
    }

    #[test]
    fn test_detect_model_family_unknown() {
        assert_eq!(detect_model_family("some-random-model"), "unknown");
    }

    #[test]
    fn test_detect_code_fence() {
        assert!(has_code_fence("```python\nprint('hello')\n```"));
        assert!(!has_code_fence("Hello, world!"));
    }

    #[test]
    fn test_detect_simple_query() {
        assert!(is_simple_query("hello"));
        assert!(is_simple_query("what is 2+2"));
        assert!(is_simple_query("capital of france"));
        assert!(!is_simple_query("write a function to sort an array"));
    }

    #[test]
    fn test_detect_repetition() {
        let repeated = "line\nline\nline\nline\nline";
        assert!(has_repetition(repeated));

        let normal = "This is a normal response\nWith multiple lines\nNo repetition";
        assert!(!has_repetition(normal));
    }

    #[test]
    fn test_analyze_code_fence_issue() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        let result = analyzer.analyze("what is 2+2", "```python\nprint(2+2)\n```");
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.issue, DetectedIssue::UnwantedCodeFence);
        assert_eq!(r.model_family, "glm");
        assert_eq!(r.category, "model:glm:code_fence");
    }

    #[test]
    fn test_analyze_output_loop() {
        let analyzer = ResponseAnalyzer::new("gemma-2-9b");
        let loopy = "line\nline\nline\nline\nline";
        let result = analyzer.analyze("tell me a joke", loopy);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.issue, DetectedIssue::OutputLoop);
        assert_eq!(r.category, "model:gemma:loop");
    }

    #[test]
    fn test_analyze_wrong_expert_glm() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        let result = analyzer.analyze(
            "hello how are you",
            "def greet():\n    import os\n    print('Hello')",
        );
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(
            r.issue,
            DetectedIssue::WrongExpert("conversation".to_string())
        );
        assert_eq!(r.category, "model:glm:wrong_expert");
    }

    #[test]
    fn test_analyze_timeout_empty() {
        let analyzer = ResponseAnalyzer::new("qwen2.5-coder");
        let result = analyzer.analyze("what is 2+2", "");
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.issue, DetectedIssue::Timeout);
        assert_eq!(r.category, "model:qwen:timeout");
    }

    #[test]
    fn test_analyze_correct_response() {
        let analyzer = ResponseAnalyzer::new("glm-4.7-flash");
        let result = analyzer.analyze("what is 2+2", "4");
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_first_answer_math() {
        assert_eq!(extract_first_answer("4\n4\n4\n4", true), "4");
        assert_eq!(extract_first_answer("42", true), "42");
        assert_eq!(extract_first_answer("", true), "");
    }

    #[test]
    fn test_extract_first_answer_nonmath() {
        let response =
            "Paris is the capital.\nIt is in France.\nParis is the capital.\nIt is in France.";
        let result = extract_first_answer(response, false);
        assert!(result.contains("Paris is the capital."));
        assert!(result.contains("It is in France."));
        // Stops at first repeated line
        assert_eq!(result.lines().count(), 2);
    }

    #[test]
    fn test_is_simple_math_query() {
        assert!(is_simple_math_query("what is 2+2"));
        assert!(is_simple_math_query("What is 10 - 3"));
        assert!(!is_simple_math_query("write a calculator program"));
        assert!(!is_simple_math_query("hello"));
    }

    #[test]
    fn test_issue_to_category() {
        assert_eq!(
            issue_to_category(&DetectedIssue::OutputLoop, "glm"),
            "model:glm:loop"
        );
        assert_eq!(
            issue_to_category(&DetectedIssue::UnwantedCodeFence, "gemma"),
            "model:gemma:code_fence"
        );
        assert_eq!(
            issue_to_category(&DetectedIssue::WrongExpert("chat".into()), "qwen"),
            "model:qwen:wrong_expert"
        );
        assert_eq!(
            issue_to_category(&DetectedIssue::Timeout, "deepseek"),
            "model:deepseek:timeout"
        );
    }

    #[ignore]
    #[test]
    fn test_analyze_live_model_code_fence() {
        if std::env::var("ARKAVO_TEST_LLM").is_err() && std::env::var("GEMINI_API_KEY").is_err() {
            eprintln!("Skipping: set ARKAVO_TEST_LLM=1 or GEMINI_API_KEY to run live model tests");
            return;
        }
        // Live model test would send a simple query and verify code fence detection
    }

    #[ignore]
    #[test]
    fn test_analyze_live_model_response() {
        if std::env::var("ARKAVO_TEST_LLM").is_err() && std::env::var("GEMINI_API_KEY").is_err() {
            eprintln!("Skipping: set ARKAVO_TEST_LLM=1 or GEMINI_API_KEY to run live model tests");
            return;
        }
        // Live model test would send a prompt and run full analysis pipeline
    }

    #[ignore]
    #[test]
    fn test_pattern_adjustment_with_history() {
        if std::env::var("ARKAVO_TEST_LLM").is_err() {
            eprintln!("Skipping: set ARKAVO_TEST_LLM=1 to run integration tests with store");
            return;
        }
        // Integration test for CRIT-012 — needs store
    }
}
