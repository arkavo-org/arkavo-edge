#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unused_async)]

pub mod architect;
pub mod classifier;
pub mod connectivity;
pub mod decision;
pub mod error;
pub mod health;
pub mod judge;
pub mod metrics;
pub mod model_discovery;
pub mod orchestrator;
pub mod prediction;
pub mod selector;
pub mod tool_request_parser;
pub mod tools;
pub mod validator;

pub use architect::{
    ArchitectExecutor, ArchitectPlan, ArchitectPlanner, ArchitectResult, ComplexityScore,
    ComplexityScorer, Subtask, SubtaskResult,
};
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
pub use selector::{ModelSelector, ProviderAvailability};
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
            TaskCategory::CodeGeneration => ModelChoice::LocalGemma4B,
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

    /// Check if Anthropic API is available
    pub fn is_anthropic_available(&self) -> bool {
        std::env::var("ANTHROPIC_API_KEY").is_ok()
    }

    /// Get Anthropic provider if configured
    pub fn get_anthropic_provider(
        &self,
    ) -> Option<arkavo_llm::providers::anthropic::AnthropicProvider> {
        arkavo_llm::providers::anthropic::AnthropicProvider::from_env().ok()
    }

    /// Get the list of available LLMs for status reporting
    pub fn get_available_llms(&self) -> Vec<LlmInfo> {
        let mut llms = Vec::new();

        // Check for Anthropic Claude
        if self.is_anthropic_available() {
            let model = std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-5-20250929".to_string());
            llms.push(LlmInfo {
                name: "Claude".to_string(),
                provider: "Anthropic".to_string(),
                model,
                available: true,
            });
        }

        // Check for Gemini
        if self.is_gemini_available() {
            let model = std::env::var("GEMINI_MODEL")
                .unwrap_or_else(|_| "gemini-3-pro-preview".to_string());
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

        let tools_json = match tool_registry {
            Some(registry) => {
                let detail_level = Self::detail_level_for_model(&decision.recommended_model);
                let keywords = Self::extract_keywords(task_description);

                // Use hybrid search: semantic + token-based
                let tool_infos = Self::search_tools_hybrid(registry, &keywords, detail_level).await;

                Some(arkavo_llm::McpConverter::to_anthropic_format_minimal(
                    &tool_infos,
                ))
            }
            None => None,
        };

        let provider = self
            .instantiate_provider(&decision.recommended_model)
            .await?;

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
            let tools_json = match tool_registry {
                Some(registry) => {
                    let detail_level =
                        Self::detail_level_for_model(&current_decision.recommended_model);
                    let keywords = Self::extract_keywords(task_description);

                    // Use hybrid search: semantic + token-based
                    let tool_infos =
                        Self::search_tools_hybrid(registry, &keywords, detail_level).await;

                    // Use the correct format based on the model provider
                    let json = match current_decision.recommended_model {
                        decision::ModelChoice::GeminiFlash | decision::ModelChoice::GeminiPro => {
                            arkavo_llm::McpConverter::to_gemini_format_minimal(&tool_infos)
                        }
                        _ => arkavo_llm::McpConverter::to_anthropic_format_minimal(&tool_infos),
                    };
                    Some(json)
                }
                None => None,
            };

            let provider = self
                .instantiate_provider(&current_decision.recommended_model)
                .await?;

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
                    // Use local Gemma-3 270M model for cost-free judgment
                    match judge::ResponseJudge::new_gemma_270m().await {
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

                                // Special handling for MissingToolUse - search for tools instead of upgrading model
                                if judgment.issue_type == IssueType::MissingToolUse
                                    && !judgment.suggested_keywords.is_empty()
                                {
                                    tracing::info!(
                                        "Judge detected missing tool usage, searching for: {:?}",
                                        judgment.suggested_keywords
                                    );

                                    // Return error with special marker to trigger tool search
                                    return Err(Error::ModelExecution(format!(
                                        "MISSING_TOOL_USE:{:?}",
                                        judgment.suggested_keywords
                                    )));
                                }

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
        let provider = self
            .instantiate_provider(&decision.recommended_model)
            .await?;

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
                    if let Ok(judge) = judge::ResponseJudge::new_gemma_4b().await
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

    /// Route complex tasks through architect mode
    ///
    /// Auto-detects complex multi-step tasks and:
    /// 1. Uses Opus to create a decomposed plan
    /// 2. Auto-classifies each subtask
    /// 3. Routes subtasks to appropriate models (cheaper for frontend, premium for backend)
    /// 4. Aggregates results and tracks cost savings
    pub async fn route_architect(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<architect::ArchitectResult> {
        use architect::{ArchitectExecutor, ArchitectPlanner, ComplexityScorer};

        // Step 1: Analyze complexity
        let scorer = ComplexityScorer::new();
        let complexity = scorer.analyze(task_description);

        // Step 2: Check if architect mode should activate
        if !complexity.architect_recommended {
            tracing::debug!(
                "Architect mode not recommended: {} subtasks, {} tokens",
                complexity.estimated_subtasks,
                complexity.estimated_output_tokens
            );

            // Fall back to normal quality-gated routing
            let response = self
                .route_with_quality_gate(task_description, messages, tool_registry, 3)
                .await?;

            return Ok(architect::ArchitectResult::single_task(
                response.content,
                0.0, // Would need actual cost from provider
            ));
        }

        tracing::info!(
            "Architect mode activated: {} estimated subtasks, {:.1}% estimated savings",
            complexity.estimated_subtasks,
            complexity.estimated_savings_percent
        );

        // Step 3: Generate plan using Opus
        let planner = ArchitectPlanner::new();
        let plan = planner.create_plan(task_description, complexity).await?;

        tracing::info!(
            "Created plan with {} subtasks, estimated ${:.4} (saves ${:.4})",
            plan.subtasks.len(),
            plan.architect_estimate_usd,
            plan.opus_only_estimate_usd - plan.architect_estimate_usd
        );

        // Step 4: Execute the plan
        let executor = ArchitectExecutor::new(Arc::new(self.clone_for_executor().await?));
        let result = executor.execute(&plan, messages, tool_registry).await?;

        tracing::info!(
            "Architect mode complete: actual cost ${:.4}, savings ${:.4} ({:.1}%)",
            result.actual_cost_usd,
            result.actual_savings_usd,
            result.savings_percent()
        );

        Ok(result)
    }

    /// Create a clone of Router for use in executor
    async fn clone_for_executor(&self) -> Result<Self> {
        Ok(Self {
            classifier: self.classifier.clone(),
            selector: self.selector.clone(),
            metrics: self.metrics.clone(),
            connectivity: self.connectivity.clone(),
            offline_mode: self.offline_mode,
        })
    }

    fn upgrade_model(&self, current: &ModelChoice) -> ModelChoice {
        match current {
            ModelChoice::LocalGemma270M => ModelChoice::LocalGemma4B,
            ModelChoice::LocalGemma4B => ModelChoice::LocalGemma12B,
            ModelChoice::LocalGemma12B => ModelChoice::GeminiFlash,
            ModelChoice::LocalDeepSeekCoder => ModelChoice::GeminiFlash,
            ModelChoice::GeminiFlash => ModelChoice::ClaudeSonnet,
            ModelChoice::ClaudeSonnet => ModelChoice::GeminiPro,
            ModelChoice::GeminiPro => ModelChoice::ClaudeOpus,
            ModelChoice::ClaudeOpus => ModelChoice::ClaudeOpus,
        }
    }

    async fn instantiate_provider(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        match model {
            ModelChoice::ClaudeSonnet | ModelChoice::ClaudeOpus => {
                use arkavo_llm::providers::anthropic::AnthropicProvider;
                if let Ok(provider) = AnthropicProvider::from_env() {
                    Ok(Box::new(provider))
                } else {
                    // Fallback to Gemini if available
                    #[cfg(feature = "gemini")]
                    if let Ok(provider) = arkavo_llm::GeminiProvider::new() {
                        return Ok(Box::new(provider));
                    }
                    Err(Error::ModelExecution(
                        "ANTHROPIC_API_KEY not set and no fallback available".to_string(),
                    ))
                }
            }
            #[cfg(feature = "gemini")]
            ModelChoice::GeminiFlash | ModelChoice::GeminiPro => {
                // Try to create Gemini provider, fallback to local if API key not available
                if let Ok(provider) = arkavo_llm::GeminiProvider::new() {
                    Ok(Box::new(provider))
                } else {
                    // Fallback to local model when Gemini API key is not available
                    #[cfg(feature = "llama-cpp")]
                    {
                        let model_path = model_discovery::find_gguf_model(
                            "unsloth/gemma-3-270m-it-GGUF",
                            "gemma-3-270m-it-Q4_0.gguf",
                        )
                        .await
                        .map_err(Error::ModelExecution)?;

                        let provider = arkavo_llm::LlamaCppProvider::new(
                            "gemma-3-270m-it".to_string(),
                            model_path.to_string_lossy().to_string(),
                        )
                        .map_err(|e| {
                            Error::ModelExecution(format!(
                                "Failed to create fallback local provider: {e}"
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
                let model_path = model_discovery::find_gguf_model(
                    "unsloth/gemma-3-270m-it-GGUF",
                    "gemma-3-270m-it-Q4_0.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "gemma-3-270m-it".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma4B => {
                let model_path = model_discovery::find_gguf_model(
                    "unsloth/gemma-3-4b-it-GGUF",
                    "gemma-3-4b-it-Q4_0.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "gemma-3-4b-it".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalGemma12B => {
                let model_path = model_discovery::find_gguf_model(
                    "unsloth/gemma-3-12b-it-GGUF",
                    "gemma-3-12b-it-Q4_0.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "gemma-3-12b-it".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create LlamaCpp provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[cfg(feature = "llama-cpp")]
            ModelChoice::LocalDeepSeekCoder => {
                let model_path = model_discovery::find_gguf_model(
                    "bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF",
                    "DeepSeek-Coder-V2-Lite-Instruct-Q4_K_M.gguf",
                )
                .await
                .map_err(Error::ModelExecution)?;

                let provider = arkavo_llm::LlamaCppProvider::new(
                    "deepseek-coder-v2-lite-instruct".to_string(),
                    model_path.to_string_lossy().to_string(),
                )
                .map_err(|e| {
                    Error::ModelExecution(format!("Failed to create DeepSeek-Coder provider: {e}"))
                })?;
                Ok(Box::new(provider))
            }
            #[allow(unreachable_patterns)]
            _ => Err(Error::ModelExecution(format!(
                "Model {model:?} not available (feature not enabled)"
            ))),
        }
    }

    /// Extract keywords from task description for tool search
    fn extract_keywords(task: &str) -> String {
        let words: Vec<&str> = task
            .split_whitespace()
            .filter(|w| w.len() > 2) // Allow 3-char words like "all", "get", "set"
            .filter(|w| {
                ![
                    "this", "that", "with", "have", "from", "what", "where", "the", "and", "for",
                ]
                .contains(w)
            })
            .collect();
        words.join(" ")
    }

    /// Determine detail level based on model context size
    fn detail_level_for_model(model: &decision::ModelChoice) -> arkavo_mcp_tools::DetailLevel {
        use decision::ModelChoice;
        match model {
            ModelChoice::LocalGemma270M | ModelChoice::LocalGemma4B => {
                arkavo_mcp_tools::DetailLevel::NameOnly
            }
            ModelChoice::LocalGemma12B | ModelChoice::LocalDeepSeekCoder => {
                arkavo_mcp_tools::DetailLevel::NameAndDescription
            }
            ModelChoice::GeminiFlash
            | ModelChoice::GeminiPro
            | ModelChoice::ClaudeSonnet
            | ModelChoice::ClaudeOpus => arkavo_mcp_tools::DetailLevel::FullSchema,
        }
    }

    /// Search tools using hybrid approach: semantic (if available) + token-based
    async fn search_tools_hybrid(
        registry: &arkavo_mcp_tools::ToolRegistry,
        query: &str,
        detail: arkavo_mcp_tools::DetailLevel,
    ) -> Vec<arkavo_mcp_tools::MinimalToolInfo> {
        // See #383: Add semantic search with local model when llama-cpp feature is enabled
        registry.search_tools(query, detail)
    }
}

/// Information about an available LLM
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
