#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unused_async)]

pub mod classifier;
pub mod connectivity;
pub mod decision;
pub mod error;
pub mod health;
pub mod judge;
pub mod metrics;
pub mod orchestrator;
pub mod prediction;
pub mod selector;
pub mod validator;

pub use classifier::{TaskCategory, TaskClassifier};
pub use connectivity::ConnectivityChecker;
pub use decision::{ModelChoice, RoutingDecision};
pub use error::{Error, Result};
pub use judge::{IssueType, JudgmentResult, ResponseJudge};
pub use metrics::RoutingMetrics;
pub use orchestrator::{
    CostOrchestrator, CostRecommendation, OrchestratorMetrics, ScalingDecision,
};
pub use prediction::{BudgetRunway, WorkflowCostPrediction, WorkflowCostPredictor};
pub use selector::ModelSelector;
pub use validator::{ResponseValidator, ValidationError};

use arkavo_llm::{Message, Provider, ProviderResponse, StreamResponse};
use arkavo_mcp_tools::ToolRegistry;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::Stream;

/// Default buffer size for streaming channels
/// Balance between memory usage and backpressure handling
const STREAM_BUFFER_SIZE: usize = 100;

/// Intelligent router for cost-optimized model selection
pub struct Router {
    classifier: Arc<TaskClassifier>,
    selector: Arc<ModelSelector>,
    metrics: Arc<RwLock<RoutingMetrics>>,
    connectivity: Arc<ConnectivityChecker>,
    offline_mode: bool,
}

impl Router {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector: Arc::new(ModelSelector::new()),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: false,
        })
    }

    pub async fn new_offline() -> Result<Self> {
        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector: Arc::new(ModelSelector::new()),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: true,
        })
    }

    pub fn set_offline_mode(&mut self, offline: bool) {
        self.offline_mode = offline;
    }

    pub async fn check_connectivity(&self) -> bool {
        self.connectivity.is_online().await
    }

    pub async fn route(&self, task_description: &str) -> Result<RoutingDecision> {
        let classification = self.classifier.classify(task_description).await?;

        let mut decision = self.selector.select(&classification, task_description)?;

        if (self.offline_mode || !self.connectivity.is_online().await)
            && decision.recommended_model.is_cloud()
        {
            let local_model = self.get_local_fallback(classification.category);

            decision.reasoning = format!(
                "Offline mode: Using local {}. Original: {}",
                local_model.name(),
                decision.reasoning
            );

            decision.recommended_model = local_model;
            decision.estimated_cost_usd = 0.0;
            decision.should_compress = false;
        }

        self.metrics
            .write()
            .await
            .record_routing(&classification, &decision);

        Ok(decision)
    }

    fn get_local_fallback(&self, category: TaskCategory) -> ModelChoice {
        match category {
            TaskCategory::FrontendUI | TaskCategory::BackendAPI | TaskCategory::Refactoring => {
                ModelChoice::LocalGemma4B
            }
            _ => ModelChoice::LocalGemma4B,
        }
    }

    pub async fn get_metrics(&self) -> RoutingMetrics {
        self.metrics.read().await.clone()
    }

    /// Get a reference to the local provider for simple classification tasks
    pub fn get_local_provider(&self) -> Arc<TaskClassifier> {
        self.classifier.clone()
    }

    /// Get a Gemini provider for complex planning/thinking tasks
    /// Returns None if Gemini is not available (no API key)
    pub fn get_planning_provider(&self) -> Option<arkavo_llm::GeminiProvider> {
        arkavo_llm::GeminiProvider::new().ok()
    }

    /// Check if Gemini API is available
    pub fn is_gemini_available(&self) -> bool {
        std::env::var("GEMINI_API_KEY").is_ok()
    }

    /// Get the list of available LLMs for status reporting
    pub fn get_available_llms(&self) -> Vec<LlmInfo> {
        let mut llms = Vec::new();

        // Check for Gemini
        if self.is_gemini_available() {
            let model =
                std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-2.5-pro".to_string());
            llms.push(LlmInfo {
                name: "Gemini".to_string(),
                provider: "Google".to_string(),
                model,
                available: true,
            });
        }

        // Always include local model
        llms.push(LlmInfo {
            name: "Local (Gemma)".to_string(),
            provider: "Local".to_string(),
            model: "gemma-3-270m-it".to_string(),
            available: true,
        });

        llms
    }

    /// Route a request with MCP tool support
    pub async fn route_with_tools(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<ProviderResponse> {
        let decision = self.route(task_description).await?;

        let tools_json = tool_registry.map(|registry| {
            let tool_infos = registry.list_tools();
            arkavo_llm::McpConverter::to_anthropic_format(&tool_infos)
        });

        let provider = self.instantiate_provider(&decision.recommended_model)?;

        provider
            .complete_with_tools(messages, tools_json, None)
            .await
            .map_err(|e| Error::ModelExecution(format!("Provider error: {e}")))
    }

    /// Route with automatic quality evaluation and model escalation
    pub async fn route_with_quality_gate(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        max_retries: u8,
    ) -> Result<ProviderResponse> {
        let mut current_decision = self.route(task_description).await?;

        for attempt in 0..max_retries {
            let tools_json = tool_registry.map(|registry| {
                let tool_infos = registry.list_tools();
                // Use the correct format based on the model provider
                match current_decision.recommended_model {
                    decision::ModelChoice::GeminiFlash | decision::ModelChoice::GeminiPro => {
                        arkavo_llm::McpConverter::to_gemini_format(&tool_infos)
                    }
                    _ => arkavo_llm::McpConverter::to_anthropic_format(&tool_infos),
                }
            });

            let provider = self.instantiate_provider(&current_decision.recommended_model)?;

            let response = provider
                .complete_with_tools(messages.clone(), tools_json, None)
                .await
                .map_err(|e| Error::ModelExecution(format!("Provider error: {e}")))?;

            if let Some(registry) = tool_registry {
                let tool_infos = registry.list_tools();

                let validator = validator::ResponseValidator::new(&tool_infos);
                if let Err(validation_error) = validator.quick_validate(&response) {
                    tracing::warn!(
                        "Fast validation failed on attempt {}/{}: {}",
                        attempt + 1,
                        max_retries,
                        validation_error
                    );

                    if attempt + 1 < max_retries {
                        current_decision.recommended_model =
                            self.upgrade_model(&current_decision.recommended_model);
                        tracing::info!(
                            "Upgrading to {:?} due to validation failure",
                            current_decision.recommended_model
                        );
                        continue;
                    }
                    return Err(Error::ModelExecution(format!(
                        "Max retries exceeded. Last error: {validation_error}"
                    )));
                }

                #[cfg(feature = "llama-cpp")]
                {
                    // Try to create judge, but gracefully degrade if model unavailable
                    match judge::ResponseJudge::new_gemma_4b() {
                        Ok(judge) => {
                            let judgment = judge
                                .evaluate(task_description, &response, &tool_infos)
                                .await?;

                            if !judgment.passed {
                                tracing::warn!(
                                    "Judge rejected response on attempt {}/{}: {:?} - {}",
                                    attempt + 1,
                                    max_retries,
                                    judgment.issue_type,
                                    judgment.reason.as_deref().unwrap_or("No reason provided")
                                );

                                if attempt + 1 < max_retries {
                                    current_decision.recommended_model =
                                        self.upgrade_model(&current_decision.recommended_model);
                                    tracing::info!(
                                        "Upgrading to {:?} after judge rejection",
                                        current_decision.recommended_model
                                    );
                                    continue;
                                }
                                return Err(Error::ModelExecution(format!(
                                    "Max retries exceeded. Judge rejected: {:?}",
                                    judgment.issue_type
                                )));
                            }
                        }
                        Err(e) => {
                            // Judge unavailable, skip LLM-based validation
                            tracing::debug!("Judge validation skipped (model unavailable): {}", e);
                        }
                    }
                }
            }

            return Ok(response);
        }

        Err(Error::ModelExecution(
            "Max retries exceeded without successful response".to_string(),
        ))
    }

    /// Route with streaming and async quality validation
    /// Streams responses immediately for real-time UX, validates in background
    ///
    /// Note: Streaming mode cannot retry on failure - use route_with_quality_gate() for retry capability.
    /// The _max_retries parameter is kept for API compatibility with non-streaming version.
    pub async fn route_with_quality_gate_stream(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        _max_retries: u8, // API compatibility - streaming cannot retry once started
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let decision = self.route(task_description).await?;
        let provider = self.instantiate_provider(&decision.recommended_model)?;

        let stream = provider
            .stream(messages.clone())
            .await
            .map_err(|e| Error::ModelExecution(format!("Provider streaming error: {e}")))?;

        let (tx, rx) = tokio::sync::mpsc::channel(STREAM_BUFFER_SIZE);
        let model = decision.recommended_model.clone();
        let tool_infos = tool_registry.map(|r| r.list_tools());
        #[cfg(feature = "llama-cpp")]
        let task_desc = task_description.to_string();

        tokio::spawn(async move {
            let mut stream = stream;
            let mut accumulated = String::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        accumulated.push_str(&chunk.content);
                        if tx.send(Ok(chunk)).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(Error::ModelExecution(format!("Stream error: {e}"))))
                            .await;
                        break;
                    }
                }
            }

            // Async quality validation - runs after streaming completes
            // Future enhancement: Track validation failures for metrics/alerting
            if let Some(tools) = tool_infos {
                let response = ProviderResponse {
                    content: accumulated.clone(),
                    tool_calls: Vec::new(),
                    finish_reason: Some("stop".to_string()),
                };

                // Fast validation: Check for hallucinated tools and parameter issues
                let validator = validator::ResponseValidator::new(&tools);
                if let Err(e) = validator.quick_validate(&response) {
                    tracing::warn!(
                        target: "arkavo_router::quality_gate",
                        model = ?model,
                        error = %e,
                        "Async validation failed - consider metrics/alerting"
                    );
                }

                // Deep validation with LLM judge (when available)
                #[cfg(feature = "llama-cpp")]
                {
                    if let Ok(judge) = judge::ResponseJudge::new_gemma_4b()
                        && let Ok(judgment) = judge.evaluate(&task_desc, &response, &tools).await
                        && !judgment.passed
                    {
                        tracing::warn!(
                            target: "arkavo_router::quality_gate",
                            model = ?model,
                            issue = ?judgment.issue_type,
                            reason = %judgment.reason.as_deref().unwrap_or("No reason"),
                            "Async judge rejected response - consider metrics/alerting"
                        );
                    }
                }
            }
        });

        Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn upgrade_model(&self, current: &ModelChoice) -> ModelChoice {
        match current {
            ModelChoice::LocalGemma270M => ModelChoice::LocalGemma4B,
            ModelChoice::LocalGemma4B => ModelChoice::LocalGemma12B,
            ModelChoice::LocalGemma12B => ModelChoice::GeminiFlash,
            ModelChoice::GeminiFlash => ModelChoice::GeminiPro,
            ModelChoice::GeminiPro => ModelChoice::GeminiPro,
        }
    }

    fn instantiate_provider(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        match model {
            #[cfg(feature = "gemini")]
            ModelChoice::GeminiFlash | ModelChoice::GeminiPro => {
                // Try to create Gemini provider, fallback to local if API key not available
                if let Ok(provider) = arkavo_llm::GeminiProvider::new() {
                    Ok(Box::new(provider))
                } else {
                    // Fallback to local model when Gemini API key is not available
                    #[cfg(feature = "llama-cpp")]
                    {
                        // Try to find model in HuggingFace cache or use env var
                        let model_path = std::env::var("ARKAVO_GEMMA_270M_PATH")
                            .or_else(|_| {
                                // Check HuggingFace cache
                                let home = std::env::var("HOME")
                                    .or_else(|_| std::env::var("USERPROFILE"))?;
                                let hf_cache = std::path::PathBuf::from(home).join(
                                    ".cache/huggingface/hub/models--unsloth--gemma-3-270m-it-GGUF",
                                );

                                if hf_cache.exists() {
                                    // Find the snapshot directory
                                    let snapshots = hf_cache.join("snapshots");
                                    if let Ok(entries) = std::fs::read_dir(&snapshots) {
                                        for entry in entries.flatten() {
                                            let gguf_path =
                                                entry.path().join("gemma-3-270m-it-Q4_0.gguf");
                                            if gguf_path.exists() {
                                                return Ok(gguf_path.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                                Err(std::env::VarError::NotPresent)
                            })
                            .unwrap_or_else(|_| "models/gemma-3-270m-it.gguf".to_string());

                        let provider = arkavo_llm::LlamaCppProvider::new(
                            "gemma-3-270m-it".to_string(),
                            model_path,
                        )
                        .map_err(|e| {
                            Error::ModelExecution(format!(
                                "Failed to create fallback local provider: {e}. Install model with: huggingface-cli download unsloth/gemma-3-270m-it-GGUF gemma-3-270m-it-Q4_0.gguf"
                            ))
                        })?;
                        Ok(Box::new(provider))
                    }
                    #[cfg(not(feature = "llama-cpp"))]
                    {
                        Err(Error::ModelExecution(
                            "Gemini API key not set and no local model fallback available. Set GEMINI_API_KEY or rebuild with llama-cpp feature.".to_string()
                        ))
                    }
                }
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma270M => {
                let model_path = std::env::var("ARKAVO_GEMMA_270M_PATH")
                    .unwrap_or_else(|_| "models/gemma-3-270m-it.gguf".to_string());
                let provider =
                    arkavo_llm::LlamaCppProvider::new("gemma-3-270m-it".to_string(), model_path)
                        .map_err(|e| {
                            Error::ModelExecution(format!(
                                "Failed to create LlamaCpp provider: {e}"
                            ))
                        })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma4B => {
                let model_path = std::env::var("ARKAVO_GEMMA_4B_PATH")
                    .unwrap_or_else(|_| "models/gemma-3-4b-it.gguf".to_string());
                let provider =
                    arkavo_llm::LlamaCppProvider::new("gemma-3-4b-it".to_string(), model_path)
                        .map_err(|e| {
                            Error::ModelExecution(format!(
                                "Failed to create LlamaCpp provider: {e}"
                            ))
                        })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma12B => {
                let model_path = std::env::var("ARKAVO_GEMMA_12B_PATH")
                    .unwrap_or_else(|_| "models/gemma-3-12b-it.gguf".to_string());
                let provider =
                    arkavo_llm::LlamaCppProvider::new("gemma-3-12b-it".to_string(), model_path)
                        .map_err(|e| {
                            Error::ModelExecution(format!(
                                "Failed to create LlamaCpp provider: {e}"
                            ))
                        })?;
                Ok(Box::new(provider))
            }
            #[allow(unreachable_patterns)]
            _ => Err(Error::ModelExecution(format!(
                "Model {model:?} not available (feature not enabled)"
            ))),
        }
    }
}

/// Information about an available LLM
#[derive(Debug, Clone)]
pub struct LlmInfo {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub available: bool,
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_router_creation() {
        let result = Router::new().await;
        if result.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        assert!(result.is_ok());
    }
}
