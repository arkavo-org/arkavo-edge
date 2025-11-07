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
                    decision::ModelChoice::LocalGemma270M
                    | decision::ModelChoice::LocalGemma4B
                    | decision::ModelChoice::LocalGemma12B => {
                        // Local models use Anthropic format (or could use XML)
                        arkavo_llm::McpConverter::to_anthropic_format(&tool_infos)
                    }
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

    /// Route with streaming and hybrid quality validation
    /// Streams responses immediately for real-time UX, validates after completion,
    /// and retries with escalated model if validation fails
    pub async fn route_with_quality_gate_stream(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        max_retries: u8,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let mut current_decision = self.route(task_description).await?;

        for attempt in 0..max_retries {
            let provider = self.instantiate_provider(&current_decision.recommended_model)?;

            let mut stream = provider
                .stream(messages.clone())
                .await
                .map_err(|e| Error::ModelExecution(format!("Provider streaming error: {e}")))?;

            let mut accumulated_content = String::new();
            let mut chunks = Vec::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        accumulated_content.push_str(&chunk.content);
                        chunks.push(Ok(chunk));
                    }
                    Err(e) => {
                        if attempt + 1 < max_retries {
                            current_decision.recommended_model =
                                self.upgrade_model(&current_decision.recommended_model);
                            tracing::warn!(
                                "Stream error on attempt {}/{}, upgrading to {:?}: {}",
                                attempt + 1,
                                max_retries,
                                current_decision.recommended_model,
                                e
                            );
                            break;
                        }
                        return Err(Error::ModelExecution(format!("Stream error: {e}")));
                    }
                }
            }

            if chunks.is_empty() {
                continue;
            }

            let synthetic_response = ProviderResponse {
                content: accumulated_content,
                tool_calls: Vec::new(),
                finish_reason: Some("stop".to_string()),
            };

            if let Some(registry) = tool_registry {
                let tool_infos = registry.list_tools();
                let validator = validator::ResponseValidator::new(&tool_infos);

                if let Err(validation_error) = validator.quick_validate(&synthetic_response) {
                    tracing::warn!(
                        "Post-stream validation failed on attempt {}/{}: {}",
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
                                .evaluate(task_description, &synthetic_response, &tool_infos)
                                .await?;

                            if !judgment.passed {
                                tracing::warn!(
                                    "Judge rejected streamed response on attempt {}/{}: {:?} - {}",
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

            return Ok(Box::new(tokio_stream::iter(chunks)));
        }

        Err(Error::ModelExecution(
            "Max retries exceeded without successful stream".to_string(),
        ))
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
                let provider = arkavo_llm::GeminiProvider::new().map_err(|e| {
                    Error::ModelExecution(format!("Failed to create Gemini provider: {e}"))
                })?;
                Ok(Box::new(provider))
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
