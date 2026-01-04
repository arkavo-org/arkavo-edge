//! RLM (Recursive Language Model) integration for chat command.
//!
//! Enables handling of large contexts that exceed model context windows
//! by decomposing them into semantic chunks and providing probe/search tools.

use arkavo_router::rlm::{
    RlmConfig, RlmDecompositionResult, RlmProbeResult, RlmSearchResult, SharedRlmManager,
};
use tracing::info;

/// RLM session state for tracking active decompositions.
pub struct RlmSession {
    manager: SharedRlmManager,
    active_manifest: Option<RlmDecompositionResult>,
    model_context_size: usize,
}

impl RlmSession {
    /// Create a new RLM session with model context size.
    pub fn new(model_context_size: usize) -> Self {
        let config = RlmConfig {
            activation_threshold_ratio: 0.7,
            max_depth: 1,
            max_probe_chunks: 10,
            parallel_processing: true,
        };

        Self {
            manager: arkavo_router::rlm::create_rlm_manager_with_config(config),
            active_manifest: None,
            model_context_size,
        }
    }

    /// Create with custom configuration.
    pub fn with_config(model_context_size: usize, config: RlmConfig) -> Self {
        Self {
            manager: arkavo_router::rlm::create_rlm_manager_with_config(config),
            active_manifest: None,
            model_context_size,
        }
    }

    /// Check if RLM mode should be activated for given input.
    pub fn should_activate(&self, input_tokens: usize) -> bool {
        self.manager
            .should_activate(input_tokens, self.model_context_size)
    }

    /// Decompose large input context for RLM processing.
    pub async fn decompose(&mut self, context: &str) -> Result<RlmDecompositionResult, String> {
        info!(
            context_len = context.len(),
            model_context = self.model_context_size,
            "Activating RLM mode for large context"
        );

        let result = self
            .manager
            .decompose(context)
            .await
            .map_err(|e| e.to_string())?;

        info!(
            manifest_id = %result.manifest_id,
            chunk_count = result.chunk_count,
            total_tokens = result.total_tokens,
            "Context decomposed into chunks"
        );

        self.active_manifest = Some(result.clone());
        Ok(result)
    }

    /// Get the active decomposition result.
    pub fn active_decomposition(&self) -> Option<&RlmDecompositionResult> {
        self.active_manifest.as_ref()
    }

    /// Check if RLM mode is currently active.
    pub fn is_active(&self) -> bool {
        self.active_manifest.is_some()
    }

    /// Probe specific chunks by indices.
    pub async fn probe(&self, indices: &[usize]) -> Result<RlmProbeResult, String> {
        let manifest = self
            .active_manifest
            .as_ref()
            .ok_or("No active RLM session")?;

        self.manager
            .probe(&manifest.manifest_id, indices)
            .await
            .map_err(|e| e.to_string())
    }

    /// Search chunks by keywords.
    pub async fn search(&self, keywords: &[&str]) -> Result<RlmSearchResult, String> {
        let manifest = self
            .active_manifest
            .as_ref()
            .ok_or("No active RLM session")?;

        self.manager
            .search(&manifest.manifest_id, keywords)
            .await
            .map_err(|e| e.to_string())
    }

    /// Generate RLM-enhanced system prompt.
    pub fn generate_system_prompt(&self) -> Option<String> {
        self.active_manifest
            .as_ref()
            .map(|result| self.manager.generate_rlm_system_prompt(result))
    }

    /// Clear the active session.
    pub async fn clear(&mut self) {
        if let Some(ref manifest) = self.active_manifest {
            self.manager.clear_manifest(&manifest.manifest_id).await;
        }
        self.active_manifest = None;
    }

    /// Get RLM statistics.
    pub async fn stats(&self) -> RlmStats {
        let internal_stats = self.manager.stats().await;
        RlmStats {
            active_manifests: internal_stats.manifest_count,
            total_chunks: internal_stats.total_chunks,
            total_tokens: internal_stats.total_tokens,
            model_context_size: self.model_context_size,
        }
    }
}

/// RLM statistics for display.
#[derive(Debug, Clone)]
pub struct RlmStats {
    pub active_manifests: usize,
    pub total_chunks: usize,
    pub total_tokens: usize,
    pub model_context_size: usize,
}

impl std::fmt::Display for RlmStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RLM Stats: {} manifests, {} chunks, {} tokens (model ctx: {})",
            self.active_manifests, self.total_chunks, self.total_tokens, self.model_context_size
        )
    }
}

/// Estimate token count for a string.
pub fn estimate_tokens(text: &str) -> usize {
    // Rough estimation: ~4 characters per token
    text.len() / 4
}

/// Get model context size from size hint.
pub fn model_context_from_hint(hint: Option<&str>, is_cloud: bool) -> usize {
    if is_cloud {
        // Cloud models have large context windows
        131_072 // 128K tokens typical for modern cloud models
    } else {
        match hint {
            Some("270M") => 2_048,
            Some("1B") => 4_096,
            Some("2B") => 8_192,
            Some("3B") => 8_192,
            Some("7B") => 8_192,
            Some("8B") => 32_768,
            Some("14B") => 32_768,
            _ => 8_192, // Default for unknown local models
        }
    }
}

/// Format RLM activation message for display.
pub fn format_activation_message(result: &RlmDecompositionResult) -> String {
    format!(
        "RLM Mode Active: {} chunks, {} total tokens\n\
         Manifest: {}\n\
         Use context_probe(indices) and context_search(keywords) to explore.",
        result.chunk_count, result.total_tokens, result.manifest_id
    )
}

/// Parse RLM tool call from assistant response.
pub fn parse_rlm_tool_call(response: &str) -> Option<RlmToolCall> {
    // Parse context_probe(manifest_id="...", indices=[...])
    if let Some(start) = response.find("context_probe(") {
        let content = &response[start..];
        if let Some(end) = content.find(')') {
            let args = &content[14..end];
            // Extract indices
            if let Some(idx_start) = args.find("indices=[") {
                let idx_content = &args[idx_start + 9..];
                if let Some(idx_end) = idx_content.find(']') {
                    let indices_str = &idx_content[..idx_end];
                    let indices: Vec<usize> = indices_str
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect();
                    if !indices.is_empty() {
                        return Some(RlmToolCall::Probe { indices });
                    }
                }
            }
        }
    }

    // Parse context_search(manifest_id="...", keywords=[...])
    if let Some(start) = response.find("context_search(") {
        let content = &response[start..];
        if let Some(end) = content.find(')') {
            let args = &content[15..end];
            // Extract keywords
            if let Some(kw_start) = args.find("keywords=[") {
                let kw_content = &args[kw_start + 10..];
                if let Some(kw_end) = kw_content.find(']') {
                    let keywords_str = &kw_content[..kw_end];
                    let keywords: Vec<String> = keywords_str
                        .split(',')
                        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !keywords.is_empty() {
                        return Some(RlmToolCall::Search { keywords });
                    }
                }
            }
        }
    }

    None
}

/// Parsed RLM tool call.
#[derive(Debug, Clone)]
pub enum RlmToolCall {
    Probe { indices: Vec<usize> },
    Search { keywords: Vec<String> },
}

impl RlmToolCall {
    /// Execute the tool call against an RLM session.
    pub async fn execute(&self, session: &RlmSession) -> Result<String, String> {
        match self {
            RlmToolCall::Probe { indices } => {
                let result = session.probe(indices).await?;
                let output: Vec<String> = result
                    .chunks
                    .iter()
                    .map(|c| format!("[Chunk {}]:\n{}", c.index, c.content))
                    .collect();
                Ok(output.join("\n\n"))
            }
            RlmToolCall::Search { keywords } => {
                let keywords_refs: Vec<&str> = keywords.iter().map(|s| s.as_str()).collect();
                let result = session.search(&keywords_refs).await?;
                if result.matches.is_empty() {
                    Ok(format!(
                        "No chunks found matching keywords: {}",
                        keywords.join(", ")
                    ))
                } else {
                    let output: Vec<String> = result
                        .matches
                        .iter()
                        .map(|m| format!("[Chunk {} - Match]:\n{}", m.index, m.content))
                        .collect();
                    Ok(format!(
                        "Found {} matching chunks:\n\n{}",
                        result.match_count,
                        output.join("\n\n")
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello world"), 2); // 11 chars / 4 = 2
        assert_eq!(estimate_tokens("a".repeat(100).as_str()), 25);
    }

    #[test]
    fn test_model_context_from_hint() {
        assert_eq!(model_context_from_hint(Some("270M"), false), 2_048);
        assert_eq!(model_context_from_hint(Some("7B"), false), 8_192);
        assert_eq!(model_context_from_hint(Some("14B"), false), 32_768);
        assert_eq!(model_context_from_hint(None, true), 131_072);
    }

    #[test]
    fn test_parse_probe_tool_call() {
        let response = r#"Let me probe those chunks.
context_probe(manifest_id="abc123", indices=[0, 2, 5])
Then I'll analyze them."#;

        let call = parse_rlm_tool_call(response);
        assert!(call.is_some());
        if let Some(RlmToolCall::Probe { indices }) = call {
            assert_eq!(indices, vec![0, 2, 5]);
        } else {
            panic!("Expected Probe call");
        }
    }

    #[test]
    fn test_parse_search_tool_call() {
        let response = r#"I'll search for authentication.
context_search(manifest_id="abc123", keywords=["auth", "login"])
Looking at results."#;

        let call = parse_rlm_tool_call(response);
        assert!(call.is_some());
        if let Some(RlmToolCall::Search { keywords }) = call {
            assert_eq!(keywords, vec!["auth", "login"]);
        } else {
            panic!("Expected Search call");
        }
    }

    #[tokio::test]
    async fn test_rlm_session_creation() {
        let session = RlmSession::new(8192);
        assert!(!session.is_active());
        assert!(!session.should_activate(1000)); // Below threshold
        assert!(session.should_activate(7000)); // Above 70% of 8192
    }
}
