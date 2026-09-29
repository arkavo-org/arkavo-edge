//! RLM Bridge - connects MCP context tools to arkavo-router's RlmContextManager.
//!
//! Implements the RlmOperations trait to enable context_probe, context_search,
//! and context_decompose MCP tools to work with the RLM system.

use arkavo_mcp_tools::context_tools::{ChunkPreview, DecomposeResult, RlmOperations};
use arkavo_router::rlm::SharedRlmManager;
use async_trait::async_trait;
use tracing::{debug, info};

/// Bridge between MCP tools and RLM manager.
pub struct RlmBridge {
    manager: SharedRlmManager,
}

impl RlmBridge {
    /// Create a new RLM bridge with the given manager.
    pub fn new(manager: SharedRlmManager) -> Self {
        Self { manager }
    }

    /// Create with a new default manager.
    pub fn with_default_manager() -> Self {
        Self {
            manager: arkavo_router::rlm::create_rlm_manager(),
        }
    }

    /// Get the underlying manager reference.
    pub fn manager(&self) -> &SharedRlmManager {
        &self.manager
    }

    /// Check if RLM mode should activate for given token count.
    pub fn should_activate(&self, input_tokens: usize, model_context_size: usize) -> bool {
        self.manager
            .should_activate(input_tokens, model_context_size)
    }

    /// Generate system prompt for RLM mode.
    pub fn generate_system_prompt(
        &self,
        result: &arkavo_router::rlm::RlmDecompositionResult,
    ) -> String {
        self.manager.generate_rlm_system_prompt(result)
    }
}

#[async_trait]
impl RlmOperations for RlmBridge {
    async fn decompose(&self, content: &str) -> Result<DecomposeResult, String> {
        info!(
            content_len = content.len(),
            "RLM bridge: decomposing context"
        );

        let result = self
            .manager
            .decompose(content)
            .await
            .map_err(|e| e.to_string())?;

        let previews: Vec<ChunkPreview> = result
            .chunk_previews
            .iter()
            .map(|p| ChunkPreview {
                index: p.index,
                tokens: p.tokens,
                preview: p.preview.clone(),
                hints: p.hints.clone(),
            })
            .collect();

        debug!(
            manifest_id = %result.manifest_id,
            chunk_count = result.chunk_count,
            "RLM bridge: decomposition complete"
        );

        Ok(DecomposeResult {
            manifest_id: result.manifest_id,
            chunk_count: result.chunk_count,
            total_tokens: result.total_tokens,
            previews,
        })
    }

    async fn probe(
        &self,
        manifest_id: &str,
        indices: &[usize],
    ) -> Result<Vec<(usize, String)>, String> {
        debug!(manifest_id, indices = ?indices, "RLM bridge: probing chunks");

        let result = self
            .manager
            .probe(manifest_id, indices)
            .await
            .map_err(|e| e.to_string())?;

        Ok(result
            .chunks
            .into_iter()
            .map(|c| (c.index, c.content))
            .collect())
    }

    async fn search(
        &self,
        manifest_id: &str,
        keywords: &[&str],
    ) -> Result<Vec<(usize, String)>, String> {
        debug!(manifest_id, keywords = ?keywords, "RLM bridge: searching chunks");

        let result = self
            .manager
            .search(manifest_id, keywords)
            .await
            .map_err(|e| e.to_string())?;

        Ok(result
            .matches
            .into_iter()
            .map(|m| (m.index, m.content))
            .collect())
    }
}

/// Estimate token count for text (rough approximation).
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Context a cloud model is planned against.
const CLOUD_CONTEXT_TOKENS: usize = 131_072;

/// Get the effective model context size based on model hint.
///
/// For a local model this is the planning budget for its size, bounded by
/// the window the local engine actually allocates in this process, so a
/// prompt the planner accepts is a prompt the model can decode. The window
/// follows the `ARKAVO_N_CTX` override and the device defaults because it
/// comes from the loader's own sizing, not from a copy of it.
///
/// Resolves full model names (e.g. "qwen3.5-27b") via `ModelChoice::from_name`
/// before falling back to bare suffix matching (e.g. "7B").
pub fn model_context_size(model_hint: Option<&str>, is_cloud: bool) -> usize {
    if is_cloud {
        return CLOUD_CONTEXT_TOKENS;
    }
    within_window(planning_budget(model_hint), local_window(model_hint))
}

/// A budget can be smaller than the window, never larger.
fn within_window(budget: usize, window: Option<usize>) -> usize {
    window.map_or(budget, |window| budget.min(window))
}

/// Tokens a local model of this size is planned against, before the
/// engine's window is taken into account.
fn planning_budget(model_hint: Option<&str>) -> usize {
    if let Some(hint) = model_hint
        && let Some(choice) = arkavo_router::ModelChoice::from_name(hint)
    {
        return match choice {
            arkavo_router::ModelChoice::LocalGemma4_26B
            | arkavo_router::ModelChoice::LocalGemma4E2B
            | arkavo_router::ModelChoice::LocalGemma4E4B => 16_384,
            _ => match choice.capability() {
                arkavo_router::PlannerTier::Small => 2_048,
                arkavo_router::PlannerTier::Medium => 8_192,
                arkavo_router::PlannerTier::Large => 32_768,
            },
        };
    }
    // Legacy bare suffix fallback
    match model_hint {
        Some("270M") => 2_048,
        Some("1B") => 4_096,
        Some("2B" | "3B") => 8_192,
        Some("7B" | "8B") => 32_768,
        Some("14B") => 32_768,
        _ => 8_192, // Default
    }
}

/// The window the local engine gives this model in this process: the one
/// its contexts were created with when it is loaded, otherwise the most the
/// loader gives any model.
#[cfg(feature = "llama-cpp")]
fn local_window(model_hint: Option<&str>) -> Option<usize> {
    let loaded = model_hint
        .and_then(arkavo_router::ModelChoice::from_name)
        .and_then(|choice| arkavo_llm::model_registry::loaded_context_length(choice.name()));
    Some(loaded.unwrap_or_else(arkavo_llama_cpp::largest_configured_context_length) as usize)
}

/// A build without the local engine allocates no window, so the budget
/// stands as it is.
#[cfg(not(feature = "llama-cpp"))]
fn local_window(_model_hint: Option<&str>) -> Option<usize> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello"), 1);
        assert_eq!(estimate_tokens("hello world test"), 4);
    }

    #[test]
    fn test_planning_budget() {
        assert_eq!(planning_budget(Some("270M")), 2_048);
        assert_eq!(planning_budget(Some("7B")), 32_768);
        assert_eq!(planning_budget(None), 8_192);
        assert_eq!(model_context_size(None, true), 131_072);
    }

    #[test]
    fn test_planning_budget_full_names() {
        assert_eq!(planning_budget(Some("qwen3.5-27b")), 32_768);
        assert_eq!(planning_budget(Some("glm-4.7-flash")), 32_768);
        assert_eq!(planning_budget(Some("ministral-8b")), 32_768);
        assert_eq!(planning_budget(Some("ministral-3b")), 8_192);
        assert_eq!(planning_budget(Some("qwen3.5-0.8b")), 2_048);
        assert_eq!(planning_budget(Some("gemma-4-26b-a4b")), 16_384);
        assert_eq!(planning_budget(Some("gemma-4-e2b")), 16_384);
        assert_eq!(planning_budget(Some("gemma-4-e4b")), 16_384);
    }

    /// Regression: the large tier was planned at 32,768 tokens while the
    /// loader allocates 16,384, so a prompt between the two was accepted by
    /// the planner and then failed at decode.
    #[test]
    fn a_budget_never_exceeds_the_window() {
        let large = planning_budget(Some("gemma-4-12b"));
        assert_eq!(large, 32_768);
        assert_eq!(within_window(large, Some(16_384)), 16_384);
        assert_eq!(within_window(large, Some(4_096)), 4_096);
    }

    #[test]
    fn a_window_larger_than_the_budget_does_not_raise_it() {
        assert_eq!(within_window(2_048, Some(16_384)), 2_048);
        assert_eq!(within_window(32_768, Some(65_536)), 32_768);
    }

    #[test]
    fn without_a_local_engine_the_budget_stands() {
        assert_eq!(within_window(32_768, None), 32_768);
    }

    /// The planner and the loader read the same sizing, whatever
    /// `ARKAVO_N_CTX` and the device say in the environment the test runs in.
    #[cfg(feature = "llama-cpp")]
    #[test]
    fn local_models_are_planned_within_the_loaders_window() {
        let window = arkavo_llama_cpp::largest_configured_context_length() as usize;
        for hint in [
            "gemma-4-12b",
            "qwen3.5-27b",
            "ministral-8b",
            "gemma-4-e2b",
            "7B",
        ] {
            assert_eq!(
                model_context_size(Some(hint), false),
                planning_budget(Some(hint)).min(window),
                "{hint}"
            );
        }
    }

    #[cfg(not(feature = "llama-cpp"))]
    #[test]
    fn a_build_without_the_engine_plans_by_budget() {
        assert_eq!(model_context_size(Some("qwen3.5-27b"), false), 32_768);
        assert_eq!(model_context_size(Some("270M"), false), 2_048);
    }

    #[tokio::test]
    async fn test_rlm_bridge_creation() {
        let bridge = RlmBridge::with_default_manager();
        assert!(!bridge.should_activate(1000, 8192));
        assert!(bridge.should_activate(6000, 8192)); // 70% of 8192
    }
}
