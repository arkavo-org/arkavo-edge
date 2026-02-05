//! RLM (Recursive Language Model) activation detection
//!
//! Detects when input context exceeds model capacity thresholds,
//! triggering RLM mode for distributed processing.

use std::sync::atomic::{AtomicBool, Ordering};

/// Debug flag for RLM detection
static SHOW_DEBUG: AtomicBool = AtomicBool::new(false);

/// Set debug mode
pub fn set_debug(enabled: bool) {
    SHOW_DEBUG.store(enabled, Ordering::SeqCst);
}

/// Check if debug mode is enabled
pub fn is_debug() -> bool {
    SHOW_DEBUG.load(Ordering::Relaxed)
}

/// Activation threshold percentage (70% of context window)
const RLM_ACTIVATION_THRESHOLD: f64 = 0.70;

/// Estimate token count from content
pub fn estimate_tokens(content: &str) -> usize {
    // Rough approximation: ~4 characters per token
    content.len() / 4
}

/// Get model context size based on model hint
pub fn model_context_size(model_hint: Option<&str>, is_cloud: bool) -> usize {
    if is_cloud {
        // Cloud models have large context windows
        return 128_000;
    }

    match model_hint {
        Some(hint) => {
            let lower = hint.to_lowercase();
            if lower.contains("270m") {
                2048
            } else if lower.contains("1b") {
                4096
            } else if lower.contains("3b")
                || lower.contains("7b")
                || lower.contains("8b")
            {
                8192
            } else if lower.contains("13b") || lower.contains("14b") {
                16384
            } else if lower.contains("70b") {
                32768
            } else if lower.contains("claude") || lower.contains("gpt") {
                128_000
            } else {
                // Default for unknown models
                4096
            }
        }
        None => 4096, // Conservative default
    }
}

/// Check if RLM mode should activate based on input size
///
/// Returns an activation hint message if RLM mode should be used,
/// None if the input fits within the model's context window.
pub fn check_rlm_activation(
    content: &str,
    model_hint: Option<&str>,
    is_cloud: bool,
) -> Option<String> {
    let input_tokens = estimate_tokens(content);
    let context_size = model_context_size(model_hint, is_cloud);
    let threshold = (context_size as f64 * RLM_ACTIVATION_THRESHOLD) as usize;

    if input_tokens > threshold {
        if is_debug() {
            eprintln!(
                "[DEBUG] RLM mode would activate: {} tokens > {}% of {} context window",
                input_tokens,
                (RLM_ACTIVATION_THRESHOLD * 100.0) as u32,
                context_size
            );
        }
        Some(format!(
            "Note: Input context ({} tokens) exceeds {}% of model context ({} tokens). \
             Consider using 'arkavo task' with mesh agents for optimal RLM processing.",
            input_tokens,
            (RLM_ACTIVATION_THRESHOLD * 100.0) as u32,
            context_size
        ))
    } else {
        None
    }
}

/// Calculate what percentage of context window is used
pub fn context_usage_percentage(content: &str, model_hint: Option<&str>, is_cloud: bool) -> f64 {
    let input_tokens = estimate_tokens(content);
    let context_size = model_context_size(model_hint, is_cloud);
    (input_tokens as f64 / context_size as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_rlm_below_threshold() {
        let content = "What is 2+2?";
        let activation = check_rlm_activation(content, Some("gemma-2b"), false);
        assert!(activation.is_none());
    }

    #[test]
    fn test_check_rlm_above_threshold() {
        // Create content that exceeds 70% of 2048 tokens (~1433 tokens)
        // At ~4 chars per token, we need ~5732 chars
        let content = "x".repeat(6000);
        let activation = check_rlm_activation(&content, Some("270M"), false);
        assert!(activation.is_some());
        assert!(activation.unwrap().contains("context"));
    }

    #[test]
    fn test_estimate_tokens() {
        let content = "Hello world"; // 11 chars
        let tokens = estimate_tokens(content);
        assert!(tokens >= 2 && tokens <= 4);
    }

    #[test]
    fn test_model_context_size_small() {
        assert_eq!(model_context_size(Some("270M"), false), 2048);
        assert_eq!(model_context_size(Some("1B"), false), 4096);
        assert_eq!(model_context_size(Some("7B"), false), 8192);
    }

    #[test]
    fn test_model_context_size_cloud() {
        let size = model_context_size(Some("claude"), true);
        assert!(size >= 128_000);
    }

    #[test]
    fn test_model_context_size_default() {
        assert_eq!(model_context_size(None, false), 4096);
    }

    #[test]
    fn test_context_usage_percentage() {
        // 100 chars = ~25 tokens, context size 4096
        let content = "a".repeat(100);
        let usage = context_usage_percentage(&content, None, false);
        assert!(usage < 1.0); // Should be well under 1%
    }

    #[test]
    fn test_debug_flag() {
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }
}
