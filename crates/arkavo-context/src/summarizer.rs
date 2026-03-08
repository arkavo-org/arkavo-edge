use crate::{Error, Result};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llm::LlamaCppProvider;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use arkavo_llm::{Message, Provider};
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use std::sync::Arc;
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
use tokio::sync::Mutex;

/// Generates concise summaries of context content for the Context Ledger.
///
/// Used to create descriptive pointers when offloading context, enabling
/// agents to understand what archived content contains without restoring it.
#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
pub struct ContextSummarizer {
    provider: Arc<Mutex<LlamaCppProvider>>,
}

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
pub struct ContextSummarizer {
    _phantom: std::marker::PhantomData<()>,
}

#[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
impl ContextSummarizer {
    /// Create a new summarizer with the given model path.
    pub async fn new(model_path: String) -> Result<Self> {
        let provider = LlamaCppProvider::new("context-summarizer".to_string(), model_path)
            .map_err(|e| Error::Model(format!("Failed to create summarizer provider: {e}")))?;

        Ok(Self {
            provider: Arc::new(Mutex::new(provider)),
        })
    }

    /// Generate a 10-20 word summary describing what the content is.
    pub async fn summarize(&self, content: &str) -> Result<String> {
        if content.is_empty() {
            return Ok("Empty content".to_string());
        }

        let prompt = self.create_summary_prompt(content);

        let messages = vec![Message::user(prompt)];

        let summary = self
            .provider
            .lock()
            .await
            .complete(messages)
            .await
            .map_err(|e| Error::Model(format!("Summarization failed: {e}")))?;

        Ok(self.clean_summary(&summary))
    }

    fn create_summary_prompt(&self, content: &str) -> String {
        // Truncate content to first 2000 chars for efficiency
        let truncated = if content.len() > 2000 {
            &content[..2000]
        } else {
            content
        };

        format!(
            r#"Summarize this content in 10-20 words. Describe WHAT it is and its key subject.
Do not explain the content, just describe it.

Examples:
- "Git diff of auth.rs adding JWT token validation"
- "Compiler errors from cargo build in user-service crate"
- "Server logs showing 500 errors on /api/payments endpoint"
- "Database schema migration adding user preferences table"

Content:
{truncated}

Summary:"#
        )
    }

    fn clean_summary(&self, summary: &str) -> String {
        // Remove quotes if present, trim whitespace
        let cleaned = summary.trim();
        let cleaned = cleaned.strip_prefix('"').unwrap_or(cleaned);
        let cleaned = cleaned.strip_suffix('"').unwrap_or(cleaned);

        // Truncate to reasonable length (max 100 chars)
        if cleaned.len() > 100 {
            format!("{}...", &cleaned[..97])
        } else {
            cleaned.to_string()
        }
    }
}

#[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
impl ContextSummarizer {
    pub async fn new(_model_path: String) -> Result<Self> {
        Err(Error::Model(
            "ContextSummarizer requires llama-cpp feature".to_string(),
        ))
    }

    pub async fn summarize(&self, _content: &str) -> Result<String> {
        Err(Error::Model(
            "ContextSummarizer requires llama-cpp feature".to_string(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_empty_content() {
        let summarizer = ContextSummarizer::new("test-model".to_string()).await;

        // Skip if no model available
        if summarizer.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let summarizer = summarizer.unwrap();

        let result = summarizer.summarize("").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Empty content");
    }

    #[test]
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    fn test_summary_prompt_format() {
        let summarizer_result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { ContextSummarizer::new("test-model".to_string()).await });

        if let Ok(summarizer) = summarizer_result {
            let prompt = summarizer.create_summary_prompt("Test content here");
            assert!(prompt.contains("10-20 words"));
            assert!(prompt.contains("Test content here"));
        }
    }

    #[test]
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    fn test_clean_summary() {
        let summarizer_result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { ContextSummarizer::new("test-model".to_string()).await });

        if let Ok(summarizer) = summarizer_result {
            // Test quote removal
            assert_eq!(
                summarizer.clean_summary("\"Git diff of auth.rs\""),
                "Git diff of auth.rs"
            );

            // Test whitespace trimming
            assert_eq!(
                summarizer.clean_summary("  Summary with spaces  "),
                "Summary with spaces"
            );

            // Test length truncation
            let long_summary = "a".repeat(150);
            let cleaned = summarizer.clean_summary(&long_summary);
            assert!(cleaned.len() <= 100);
            assert!(cleaned.ends_with("..."));
        }
    }
}
